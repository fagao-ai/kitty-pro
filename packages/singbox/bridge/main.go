// Package main builds a stable C ABI around the sing-box Go library.
package main

/*
#include <stdlib.h>
#include <stdint.h>
*/
import "C"

import (
	"bytes"
	"context"
	stdjson "encoding/json"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"os"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"
	"unsafe"

	box "github.com/sagernet/sing-box"
	"github.com/sagernet/sing-box/adapter"
	"github.com/sagernet/sing-box/common/srs"
	"github.com/sagernet/sing-box/common/urltest"
	CBox "github.com/sagernet/sing-box/constant"
	"github.com/sagernet/sing-box/experimental/clashapi"
	"github.com/sagernet/sing-box/experimental/clashapi/trafficontrol"
	"github.com/sagernet/sing-box/include"
	boxlog "github.com/sagernet/sing-box/log"
	"github.com/sagernet/sing-box/option"
	"github.com/sagernet/sing/common/json"
	"github.com/sagernet/sing/common/observable"
	singservice "github.com/sagernet/sing/service"
)

type instance struct {
	box              *box.Box
	cancel           context.CancelFunc
	trafficURL       string
	trafficAuthToken string
	logs             *bridgeLogBuffer
}

const bridgeLogLimit = 500
const probeTimeout = 5 * time.Second
const bridgeRouteEnrichmentGrace = 100 * time.Millisecond

type bridgeLogEntry struct {
	Sequence      uint64    `json:"sequence"`
	Timestamp     string    `json:"timestamp"`
	Level         string    `json:"level"`
	Message       string    `json:"message"`
	OutboundChain []string  `json:"outbound_chain,omitempty"`
	SourceIP      string    `json:"source_ip,omitempty"`
	recordedAt    time.Time `json:"-"`
}

type bridgeLogBatch struct {
	NextCursor uint64           `json:"next_cursor"`
	Entries    []bridgeLogEntry `json:"entries"`
}

type probeResult struct {
	Tag       string  `json:"tag"`
	LatencyMS *uint64 `json:"latency_ms,omitempty"`
	Error     string  `json:"error,omitempty"`
}

type bridgeLogBuffer struct {
	sync.Mutex
	enabled       atomic.Bool
	nextSequence  uint64
	entries       []bridgeLogEntry
	pendingRoutes []bridgeRouteRecord
}

type bridgeRouteRecord struct {
	target        string
	outbound      string
	outboundChain []string
	sourceIP      string
	recordedAt    time.Time
}

func (b *bridgeLogBuffer) WriteMessage(level boxlog.Level, message string) {
	// Connection routing decisions are logged at info level. Keeping more
	// verbose debug/trace output would quickly evict the useful entries.
	if level > boxlog.LevelInfo {
		return
	}
	if !b.isEnabled() {
		return
	}
	b.append(boxlog.FormatLevel(level), message)
}

func (b *bridgeLogBuffer) writeFormattedMessage(message string) {
	if !b.isEnabled() {
		return
	}
	level := inferFormattedLogLevel(message)
	if level == "debug" || level == "trace" {
		return
	}
	b.append(level, message)
}

func (b *bridgeLogBuffer) append(level string, message string) {
	b.Lock()
	defer b.Unlock()
	if !b.enabled.Load() {
		return
	}
	recordedAt := time.Now()
	b.nextSequence++
	entry := bridgeLogEntry{
		Sequence:   b.nextSequence,
		Timestamp:  recordedAt.Format("15:04:05.000"),
		Level:      level,
		Message:    stripANSI(strings.TrimSpace(message)),
		recordedAt: recordedAt,
	}
	b.attachPendingRoute(&entry)
	if len(b.entries) == bridgeLogLimit {
		copy(b.entries, b.entries[1:])
		b.entries[len(b.entries)-1] = entry
	} else {
		b.entries = append(b.entries, entry)
	}
}

func (b *bridgeLogBuffer) isEnabled() bool {
	return b.enabled.Load()
}

func (b *bridgeLogBuffer) setEnabled(enabled bool) {
	b.enabled.Store(enabled)
}

func (b *bridgeLogBuffer) reset() {
	b.Lock()
	b.nextSequence = 0
	b.entries = nil
	b.pendingRoutes = nil
	b.Unlock()
}

func (b *bridgeLogBuffer) recordConnectionRoute(target string, outbound string, outboundChain []string, sourceIP string) {
	if !b.isEnabled() || target == "" || outbound == "" || len(outboundChain) == 0 {
		return
	}
	normalizedChain := make([]string, len(outboundChain))
	for index, tag := range outboundChain {
		normalizedChain[len(outboundChain)-1-index] = tag
	}
	b.Lock()
	defer b.Unlock()
	now := time.Now()
	for index := len(b.entries) - 1; index >= 0; index-- {
		entry := &b.entries[index]
		if now.Sub(entry.recordedAt) > 10*time.Second {
			break
		}
		if len(entry.OutboundChain) > 0 {
			continue
		}
		entryOutbound, entryTarget, found := parseBridgeRouteKey(entry.Message)
		if found && entryOutbound == outbound && entryTarget == target {
			entry.OutboundChain = append([]string(nil), normalizedChain...)
			entry.SourceIP = sourceIP
			return
		}
	}
	b.pendingRoutes = append(b.pendingRoutes, bridgeRouteRecord{
		target:        target,
		outbound:      outbound,
		outboundChain: normalizedChain,
		sourceIP:      sourceIP,
		recordedAt:    now,
	})
	b.prunePendingRoutes(now)
}

func (b *bridgeLogBuffer) attachPendingRoute(entry *bridgeLogEntry) {
	outbound, target, found := parseBridgeRouteKey(entry.Message)
	if !found {
		return
	}
	for index := len(b.pendingRoutes) - 1; index >= 0; index-- {
		route := b.pendingRoutes[index]
		if entry.recordedAt.Sub(route.recordedAt) > 10*time.Second {
			break
		}
		if route.outbound == outbound && route.target == target {
			entry.OutboundChain = append([]string(nil), route.outboundChain...)
			entry.SourceIP = route.sourceIP
			b.pendingRoutes = append(b.pendingRoutes[:index], b.pendingRoutes[index+1:]...)
			return
		}
	}
	b.prunePendingRoutes(entry.recordedAt)
}

func (b *bridgeLogBuffer) prunePendingRoutes(now time.Time) {
	firstRecent := 0
	for firstRecent < len(b.pendingRoutes) && now.Sub(b.pendingRoutes[firstRecent].recordedAt) > 10*time.Second {
		firstRecent++
	}
	if firstRecent > 0 {
		b.pendingRoutes = append([]bridgeRouteRecord(nil), b.pendingRoutes[firstRecent:]...)
	}
}

func parseBridgeRouteKey(message string) (outbound string, target string, found bool) {
	componentStart := strings.Index(message, "outbound/")
	if componentStart == -1 {
		return "", "", false
	}
	component := message[componentStart+len("outbound/"):]
	tagStart := strings.IndexByte(component, '[')
	if tagStart == -1 {
		return "", "", false
	}
	tagEnd := strings.IndexByte(component[tagStart+1:], ']')
	if tagEnd == -1 {
		return "", "", false
	}
	tagEnd += tagStart + 1
	outbound = strings.TrimSpace(component[tagStart+1 : tagEnd])
	detail := component[tagEnd+1:]
	targetStart := strings.Index(detail, "connection to ")
	if outbound == "" || targetStart == -1 {
		return "", "", false
	}
	target = strings.TrimSpace(detail[targetStart+len("connection to "):])
	return outbound, target, target != ""
}

func bridgeConnectionTarget(metadata *trafficontrol.TrackerMetadata) string {
	destination := metadata.Metadata.Destination
	host := metadata.Metadata.Domain
	if host == "" {
		host = destination.AddrString()
	}
	if host == "" || destination.Port == 0 {
		return ""
	}
	return net.JoinHostPort(host, strconv.Itoa(int(destination.Port)))
}

func bridgeConnectionSourceIP(metadata *trafficontrol.TrackerMetadata) string {
	return metadata.Metadata.Source.Unwrap().AddrString()
}

func (b *bridgeLogBuffer) snapshot(cursor uint64) bridgeLogBatch {
	b.Lock()
	defer b.Unlock()
	if cursor > b.nextSequence {
		cursor = 0
	}
	entries := make([]bridgeLogEntry, 0)
	nextCursor := cursor
	now := time.Now()
	for _, entry := range b.entries {
		if entry.Sequence <= cursor {
			continue
		}
		_, _, isRoute := parseBridgeRouteKey(entry.Message)
		if isRoute && len(entry.OutboundChain) == 0 && now.Sub(entry.recordedAt) < bridgeRouteEnrichmentGrace {
			break
		}
		entries = append(entries, entry)
		nextCursor = entry.Sequence
	}
	return bridgeLogBatch{
		NextCursor: nextCursor,
		Entries:    entries,
	}
}

func (b *bridgeLogBuffer) snapshotJSON(cursor uint64) ([]byte, error) {
	return stdjson.Marshal(b.snapshot(cursor))
}

func inferFormattedLogLevel(message string) string {
	message = strings.ToUpper(stripANSI(strings.TrimSpace(message)))
	for _, level := range []string{"PANIC", "FATAL", "ERROR", "WARN", "INFO", "DEBUG", "TRACE"} {
		if strings.HasPrefix(message, level) || strings.Contains(message, " "+level+"[") {
			return strings.ToLower(level)
		}
	}
	return "info"
}

func stripANSI(message string) string {
	clean := make([]byte, 0, len(message))
	for index := 0; index < len(message); index++ {
		if message[index] == 0x1b && index+1 < len(message) && message[index+1] == '[' {
			index += 2
			for index < len(message) {
				if message[index] >= 0x40 && message[index] <= 0x7e {
					break
				}
				index++
			}
			continue
		}
		clean = append(clean, message[index])
	}
	return string(clean)
}

func validateRuleSetFile(path string) error {
	content, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	_, err = srs.Read(bytes.NewReader(content), false)
	return err
}

var state = struct {
	sync.Mutex
	nextHandle uint64
	instances  map[uint64]*instance
	lastError  string
}{
	nextHandle: 1,
	instances:  make(map[uint64]*instance),
}

func setLastError(err error) {
	state.Lock()
	defer state.Unlock()
	if err == nil {
		state.lastError = ""
		return
	}
	state.lastError = err.Error()
}

func start(configContent string) (*instance, error) {
	ctx := include.Context(context.Background())
	options, err := json.UnmarshalExtendedContext[option.Options](ctx, []byte(configContent))
	if err != nil {
		return nil, err
	}
	ctx, cancel := context.WithCancel(ctx)
	logs := &bridgeLogBuffer{}
	service, err := box.New(box.Options{
		Context: ctx,
		Options: options,
	})
	if err != nil {
		cancel()
		return nil, err
	}
	if options.Experimental != nil && options.Experimental.ClashAPI != nil {
		logFactory, observableLogs := service.LogFactory().(boxlog.ObservableFactory)
		if !observableLogs {
			cancel()
			_ = service.Close()
			return nil, &bridgeError{message: "sing-box observable logs are not available"}
		}
		logSubscription, logDone, subscribeErr := logFactory.Subscribe()
		if subscribeErr != nil {
			cancel()
			_ = service.Close()
			return nil, subscribeErr
		}
		go func() {
			defer logFactory.UnSubscribe(logSubscription)
			for {
				select {
				case entry := <-logSubscription:
					logs.WriteMessage(entry.Level, entry.Message)
				case <-logDone:
					return
				}
			}
		}()
		if clashServer, ok := singservice.FromContext[adapter.ClashServer](ctx).(*clashapi.Server); ok {
			connectionSubscriber := observable.NewSubscriber[trafficontrol.ConnectionEvent](256)
			clashServer.TrafficManager().SetEventHook(connectionSubscriber)
			connectionEvents, connectionDone := connectionSubscriber.Subscription()
			go func() {
				defer connectionSubscriber.Close()
				for {
					select {
					case event := <-connectionEvents:
						if event.Type == trafficontrol.ConnectionEventNew && event.Metadata != nil {
							logs.recordConnectionRoute(
								bridgeConnectionTarget(event.Metadata),
								event.Metadata.Outbound,
								event.Metadata.Chain,
								bridgeConnectionSourceIP(event.Metadata),
							)
						}
					case <-connectionDone:
						return
					case <-ctx.Done():
						return
					}
				}
			}()
		}
	}
	if err = service.Start(); err != nil {
		cancel()
		_ = service.Close()
		return nil, err
	}
	trafficURL, trafficAuthToken := trafficEndpointFromOptions(options)
	return &instance{
		box:              service,
		cancel:           cancel,
		trafficURL:       trafficURL,
		trafficAuthToken: trafficAuthToken,
		logs:             logs,
	}, nil
}

func recoveredError(operation string, recovered any) error {
	return fmt.Errorf("%s panic: %v", operation, recovered)
}

func trafficEndpoint(configContent string) (string, string, error) {
	ctx := include.Context(context.Background())
	options, err := json.UnmarshalExtendedContext[option.Options](ctx, []byte(configContent))
	if err != nil {
		return "", "", err
	}
	trafficURL, trafficAuthToken := trafficEndpointFromOptions(options)
	return trafficURL, trafficAuthToken, nil
}

func trafficEndpointFromOptions(options option.Options) (string, string) {
	if options.Experimental == nil || options.Experimental.ClashAPI == nil {
		return "", ""
	}
	clashAPI := options.Experimental.ClashAPI
	if clashAPI.ExternalController == "" {
		return "", ""
	}
	return "http://" + clashAPI.ExternalController + "/connections", clashAPI.Secret
}

func probe(configContent string, nodeTags []string, probeURL string) (results []probeResult, err error) {
	service, err := start(configContent)
	if err != nil {
		return nil, err
	}
	defer func() {
		service.cancel()
		if closeErr := service.box.Close(); closeErr != nil && err == nil {
			err = fmt.Errorf("close sing-box probe: %w", closeErr)
		}
	}()

	results = make([]probeResult, len(nodeTags))
	var waitGroup sync.WaitGroup
	semaphore := make(chan struct{}, 100)
	for index, tag := range nodeTags {
		index := index
		tag := tag
		results[index].Tag = tag
		waitGroup.Add(1)
		go func() {
			defer waitGroup.Done()
			defer func() {
				if recovered := recover(); recovered != nil {
					results[index] = probeResult{
						Tag:   tag,
						Error: fmt.Sprintf("sing-box probe panic: %v", recovered),
					}
				}
			}()
			semaphore <- struct{}{}
			defer func() { <-semaphore }()
			results[index] = service.probeOutbound(tag, probeURL)
		}()
	}
	waitGroup.Wait()
	return results, nil
}

func (service *instance) probeOutbound(tag string, probeURL string) probeResult {
	result := probeResult{Tag: tag}
	outbound, loaded := service.box.Outbound().Outbound(tag)
	if !loaded {
		result.Error = fmt.Sprintf("probe outbound not found: %s", tag)
		return result
	}
	ctx, cancel := context.WithTimeout(context.Background(), probeTimeout)
	defer cancel()
	delay, err := urltest.URLTest(ctx, probeURL, outbound)
	if err != nil {
		result.Error = err.Error()
		return result
	}
	latency := uint64(delay)
	result.LatencyMS = &latency
	return result
}

//export kitty_singbox_probe
func kitty_singbox_probe(configContent *C.char, nodeTagsJSON *C.char, probeURL *C.char) *C.char {
	if configContent == nil || nodeTagsJSON == nil || probeURL == nil {
		setLastError(&bridgeError{message: "missing sing-box probe parameters"})
		return nil
	}
	var nodeTags []string
	if err := stdjson.Unmarshal([]byte(C.GoString(nodeTagsJSON)), &nodeTags); err != nil {
		setLastError(err)
		return nil
	}
	results, err := probe(C.GoString(configContent), nodeTags, C.GoString(probeURL))
	if err != nil {
		setLastError(err)
		return nil
	}
	payload, err := stdjson.Marshal(results)
	if err != nil {
		setLastError(err)
		return nil
	}
	setLastError(nil)
	return C.CString(string(payload))
}

//export kitty_singbox_probe_outbound
func kitty_singbox_probe_outbound(handle C.uint64_t, tag *C.char, probeURL *C.char) *C.char {
	if tag == nil || probeURL == nil {
		setLastError(&bridgeError{message: "missing sing-box outbound probe parameters"})
		return nil
	}
	state.Lock()
	service, found := state.instances[uint64(handle)]
	state.Unlock()
	if !found {
		setLastError(&bridgeError{message: "sing-box instance is not running"})
		return nil
	}
	result := service.probeOutbound(C.GoString(tag), C.GoString(probeURL))
	payload, err := stdjson.Marshal(result)
	if err != nil {
		setLastError(err)
		return nil
	}
	setLastError(nil)
	return C.CString(string(payload))
}

//export kitty_singbox_start
func kitty_singbox_start(configContent *C.char) (result C.uint64_t) {
	defer func() {
		if recovered := recover(); recovered != nil {
			setLastError(recoveredError("start sing-box", recovered))
			result = 0
		}
	}()
	if configContent == nil {
		setLastError(&bridgeError{message: "missing sing-box configuration"})
		return 0
	}
	service, err := start(C.GoString(configContent))
	if err != nil {
		setLastError(err)
		return 0
	}

	state.Lock()
	handle := state.nextHandle
	state.nextHandle++
	state.instances[handle] = service
	state.lastError = ""
	state.Unlock()
	return C.uint64_t(handle)
}

//export kitty_singbox_stop
func kitty_singbox_stop(handle C.uint64_t) (result C.int) {
	defer func() {
		if recovered := recover(); recovered != nil {
			setLastError(recoveredError("stop sing-box", recovered))
			result = 0
		}
	}()
	state.Lock()
	service, found := state.instances[uint64(handle)]
	if found {
		delete(state.instances, uint64(handle))
	}
	state.Unlock()
	if !found {
		setLastError(&bridgeError{message: "sing-box instance is not running"})
		return 0
	}

	service.cancel()
	if err := service.box.Close(); err != nil {
		setLastError(err)
		return 0
	}
	setLastError(nil)
	return 1
}

type trafficSnapshot struct {
	UploadTotal   int64                `json:"uploadTotal"`
	DownloadTotal int64                `json:"downloadTotal"`
	Connections   []stdjson.RawMessage `json:"connections"`
}

type trafficResult struct {
	UploadTotal       uint64 `json:"upload_total"`
	DownloadTotal     uint64 `json:"download_total"`
	ActiveConnections uint32 `json:"active_connections"`
}

func (service *instance) traffic() ([]byte, error) {
	if service.trafficURL == "" {
		return nil, &bridgeError{message: "traffic statistics are not enabled"}
	}
	request, err := http.NewRequest(http.MethodGet, service.trafficURL, nil)
	if err != nil {
		return nil, err
	}
	if service.trafficAuthToken != "" {
		request.Header.Set("Authorization", "Bearer "+service.trafficAuthToken)
	}
	client := &http.Client{
		Timeout: 2 * time.Second,
		Transport: &http.Transport{
			Proxy: nil,
		},
	}
	response, err := client.Do(request)
	if err != nil {
		return nil, err
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("traffic statistics endpoint returned HTTP %d", response.StatusCode)
	}
	var snapshot trafficSnapshot
	if err = stdjson.NewDecoder(response.Body).Decode(&snapshot); err != nil {
		return nil, err
	}
	if snapshot.UploadTotal < 0 || snapshot.DownloadTotal < 0 {
		return nil, &bridgeError{message: "traffic statistics contain a negative counter"}
	}
	return stdjson.Marshal(trafficResult{
		UploadTotal:       uint64(snapshot.UploadTotal),
		DownloadTotal:     uint64(snapshot.DownloadTotal),
		ActiveConnections: uint32(len(snapshot.Connections)),
	})
}

func (service *instance) selectOutbound(group string, outbound string) error {
	if service.trafficURL == "" {
		return &bridgeError{message: "proxy selection is not enabled"}
	}
	payload, err := stdjson.Marshal(map[string]string{"name": outbound})
	if err != nil {
		return err
	}
	endpoint := strings.TrimSuffix(service.trafficURL, "/connections") + "/proxies/" + url.PathEscape(group)
	request, err := http.NewRequest(http.MethodPut, endpoint, bytes.NewReader(payload))
	if err != nil {
		return err
	}
	request.Header.Set("Content-Type", "application/json")
	if service.trafficAuthToken != "" {
		request.Header.Set("Authorization", "Bearer "+service.trafficAuthToken)
	}
	client := &http.Client{
		Timeout:   2 * time.Second,
		Transport: &http.Transport{Proxy: nil},
	}
	response, err := client.Do(request)
	if err != nil {
		return err
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK && response.StatusCode != http.StatusNoContent {
		return fmt.Errorf("proxy selection endpoint returned HTTP %d", response.StatusCode)
	}
	return nil
}

//export kitty_singbox_traffic
func kitty_singbox_traffic(handle C.uint64_t) (result *C.char) {
	defer func() {
		if recovered := recover(); recovered != nil {
			setLastError(recoveredError("read sing-box traffic", recovered))
			result = nil
		}
	}()
	state.Lock()
	service, found := state.instances[uint64(handle)]
	state.Unlock()
	if !found {
		setLastError(&bridgeError{message: "sing-box instance is not running"})
		return nil
	}
	payload, err := service.traffic()
	if err != nil {
		setLastError(err)
		return nil
	}
	setLastError(nil)
	return C.CString(string(payload))
}

//export kitty_singbox_select_outbound
func kitty_singbox_select_outbound(handle C.uint64_t, group *C.char, outbound *C.char) C.int {
	if group == nil || outbound == nil {
		setLastError(&bridgeError{message: "missing proxy group selection"})
		return 0
	}
	state.Lock()
	service, found := state.instances[uint64(handle)]
	state.Unlock()
	if !found {
		setLastError(&bridgeError{message: "sing-box instance is not running"})
		return 0
	}
	if err := service.selectOutbound(C.GoString(group), C.GoString(outbound)); err != nil {
		setLastError(err)
		return 0
	}
	setLastError(nil)
	return 1
}

//export kitty_singbox_logs
func kitty_singbox_logs(handle C.uint64_t, cursor C.uint64_t) (result *C.char) {
	defer func() {
		if recovered := recover(); recovered != nil {
			setLastError(recoveredError("read sing-box logs", recovered))
			result = nil
		}
	}()
	state.Lock()
	service, found := state.instances[uint64(handle)]
	state.Unlock()
	if !found {
		setLastError(&bridgeError{message: "sing-box instance is not running"})
		return nil
	}
	payload, err := service.logs.snapshotJSON(uint64(cursor))
	if err != nil {
		setLastError(err)
		return nil
	}
	setLastError(nil)
	return C.CString(string(payload))
}

//export kitty_singbox_set_log_enabled
func kitty_singbox_set_log_enabled(handle C.uint64_t, enabled C.int) (result C.int) {
	defer func() {
		if recovered := recover(); recovered != nil {
			setLastError(recoveredError("set sing-box log collection", recovered))
			result = 0
		}
	}()
	state.Lock()
	service, found := state.instances[uint64(handle)]
	state.Unlock()
	if !found {
		setLastError(&bridgeError{message: "sing-box instance is not running"})
		return 0
	}
	service.logs.setEnabled(enabled != 0)
	setLastError(nil)
	return 1
}

//export kitty_singbox_validate_rule_set_file
func kitty_singbox_validate_rule_set_file(path *C.char) (result *C.char) {
	defer func() {
		if recovered := recover(); recovered != nil {
			result = C.CString(recoveredError("validate sing-box rule set", recovered).Error())
		}
	}()
	if path == nil {
		return C.CString("missing sing-box rule-set path")
	}
	if err := validateRuleSetFile(C.GoString(path)); err != nil {
		return C.CString(err.Error())
	}
	return nil
}

//export kitty_singbox_version
func kitty_singbox_version() *C.char {
	return C.CString(CBox.Version)
}

//export kitty_singbox_last_error
func kitty_singbox_last_error() *C.char {
	state.Lock()
	message := state.lastError
	state.Unlock()
	return C.CString(message)
}

//export kitty_singbox_free_string
func kitty_singbox_free_string(value *C.char) {
	C.free(unsafe.Pointer(value))
}

type bridgeError struct {
	message string
}

func (e *bridgeError) Error() string {
	return e.message
}

func main() {}
