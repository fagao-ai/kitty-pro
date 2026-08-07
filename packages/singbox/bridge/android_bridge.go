//go:build android

package main

/*
#include <stdlib.h>
#include <stdint.h>
*/
import "C"

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"path/filepath"
	"sync"

	"github.com/sagernet/sing-box/experimental/libbox"
)

type androidService struct {
	server   *libbox.CommandServer
	clashAPI *clashAPIClient
}

var androidRuntime = struct {
	sync.Mutex
	service *androidService
	err     error
}{}

var androidSetup = struct {
	sync.Once
	err error
}{}

var androidLogs bridgeLogBuffer

func startAndroid(configContent string, tunFD int32, dataPath string) error {
	if tunFD <= 0 {
		return errors.New("Android VPN did not provide a TUN file descriptor")
	}
	if err := setupAndroid(dataPath); err != nil {
		return err
	}
	androidLogs.reset()

	server, err := libbox.NewCommandServer(&androidCommandHandler{}, &androidPlatform{tunFD: tunFD})
	if err != nil {
		return err
	}
	if err = server.StartOrReloadService(configContent, &libbox.OverrideOptions{}); err != nil {
		server.Close()
		return err
	}

	trafficURL, trafficAuthToken, err := trafficEndpoint(configContent)
	if err != nil {
		_ = server.CloseService()
		server.Close()
		return err
	}

	androidRuntime.Lock()
	previous := androidRuntime.service
	androidRuntime.service = &androidService{
		server:   server,
		clashAPI: newClashAPIClient(trafficURL, trafficAuthToken),
	}
	androidRuntime.err = nil
	androidRuntime.Unlock()
	if previous != nil {
		previous.clashAPI.close()
		_ = previous.server.CloseService()
		previous.server.Close()
	}
	return nil
}

func setupAndroid(dataPath string) error {
	androidSetup.Do(func() {
		androidSetup.err = libbox.Setup(&libbox.SetupOptions{
			BasePath:        dataPath,
			WorkingPath:     dataPath,
			TempPath:        filepath.Join(dataPath, "tmp"),
			FixAndroidStack: true,
			LogMaxLines:     256,
			Debug:           true,
		})
	})
	if androidSetup.err != nil {
		return androidSetup.err
	}
	return nil
}

type androidProbeResult struct {
	Tag       string `json:"tag"`
	LatencyMS uint64 `json:"latency_ms,omitempty"`
	Error     string `json:"error,omitempty"`
}

func probeAndroid(configContent string, nodeTags []string, probeURL string, dataPath string) ([]androidProbeResult, error) {
	if err := setupAndroid(dataPath); err != nil {
		return nil, err
	}

	androidRuntime.Lock()
	runningService := androidRuntime.service
	androidRuntime.Unlock()
	if runningService != nil {
		return probeAndroidServer(runningService.server, nodeTags, probeURL), nil
	}

	server, err := libbox.NewCommandServer(&androidCommandHandler{}, &androidPlatform{})
	if err != nil {
		return nil, err
	}
	if err = server.StartOrReloadService(configContent, &libbox.OverrideOptions{}); err != nil {
		server.Close()
		return nil, err
	}
	defer func() {
		_ = server.CloseService()
		server.Close()
	}()

	return probeAndroidServer(server, nodeTags, probeURL), nil
}

func probeAndroidServer(server *libbox.CommandServer, nodeTags []string, probeURL string) []androidProbeResult {
	results := make([]androidProbeResult, len(nodeTags))
	instance := server.Instance()
	if instance == nil || instance.Box() == nil {
		for index, tag := range nodeTags {
			results[index] = androidProbeResult{Tag: tag, Error: "Android probe core is not running"}
		}
		return results
	}

	var waitGroup sync.WaitGroup
	semaphore := make(chan struct{}, 100)
	for index, tag := range nodeTags {
		index := index
		tag := tag
		results[index].Tag = tag
		outbound, loaded := instance.Box().Outbound().Outbound(tag)
		if !loaded {
			results[index].Error = fmt.Sprintf("Android probe outbound not found: %s", tag)
			continue
		}
		waitGroup.Add(1)
		go func() {
			defer waitGroup.Done()
			semaphore <- struct{}{}
			defer func() { <-semaphore }()
			ctx, cancel := context.WithTimeout(context.Background(), probeTimeout)
			defer cancel()
			delay, err := unifiedURLTest(ctx, probeURL, outbound)
			if err != nil {
				results[index].Error = err.Error()
				return
			}
			results[index].LatencyMS = uint64(delay)
		}()
	}
	waitGroup.Wait()
	return results
}

func stopAndroid() error {
	androidRuntime.Lock()
	service := androidRuntime.service
	androidRuntime.service = nil
	androidRuntime.err = nil
	androidRuntime.Unlock()
	if service == nil {
		return nil
	}
	service.clashAPI.close()
	err := service.server.CloseService()
	service.server.Close()
	return err
}

func androidTraffic() ([]byte, error) {
	androidRuntime.Lock()
	service := androidRuntime.service
	androidRuntime.Unlock()
	if service == nil {
		return nil, errors.New("Android VPN service is not running")
	}
	return (&instance{clashAPI: service.clashAPI}).traffic()
}

func androidSelectOutbound(group string, outbound string) error {
	androidRuntime.Lock()
	service := androidRuntime.service
	androidRuntime.Unlock()
	if service == nil {
		return errors.New("Android VPN service is not running")
	}
	return (&instance{clashAPI: service.clashAPI}).selectOutbound(group, outbound)
}

//export kitty_singbox_android_select_outbound
func kitty_singbox_android_select_outbound(group *C.char, outbound *C.char) *C.char {
	if group == nil || outbound == nil {
		return C.CString("missing proxy group selection")
	}
	if err := androidSelectOutbound(C.GoString(group), C.GoString(outbound)); err != nil {
		return C.CString(err.Error())
	}
	return nil
}

//export kitty_singbox_android_logs
func kitty_singbox_android_logs(cursor C.uint64_t) *C.char {
	result, err := androidLogs.snapshotJSON(uint64(cursor))
	if err != nil {
		return C.CString(err.Error())
	}
	return C.CString(string(result))
}

//export kitty_singbox_android_set_log_enabled
func kitty_singbox_android_set_log_enabled(enabled C.int) {
	androidLogs.setEnabled(enabled != 0)
}

type androidCommandHandler struct{}

func (*androidCommandHandler) ServiceStop() error   { return nil }
func (*androidCommandHandler) ServiceReload() error { return nil }
func (*androidCommandHandler) GetSystemProxyStatus() (*libbox.SystemProxyStatus, error) {
	return &libbox.SystemProxyStatus{Available: false, Enabled: false}, nil
}
func (*androidCommandHandler) SetSystemProxyEnabled(bool) error { return nil }
func (*androidCommandHandler) WriteDebugMessage(message string) {
	androidLogs.writeFormattedMessage(message)
}

type androidPlatform struct {
	tunFD int32
}

func (*androidPlatform) LocalDNSTransport() libbox.LocalDNSTransport { return nil }
func (*androidPlatform) UsePlatformAutoDetectInterfaceControl() bool { return false }
func (*androidPlatform) AutoDetectInterfaceControl(int32) error      { return nil }
func (p *androidPlatform) OpenTun(libbox.TunOptions) (int32, error) {
	if p.tunFD <= 0 {
		return 0, errors.New("Android TUN descriptor has already been consumed")
	}
	fd := p.tunFD
	p.tunFD = 0
	return fd, nil
}
func (*androidPlatform) UseProcFS() bool { return false }
func (*androidPlatform) FindConnectionOwner(int32, string, int32, string, int32) (*libbox.ConnectionOwner, error) {
	return nil, errors.New("Android process lookup is not configured")
}
func (*androidPlatform) StartDefaultInterfaceMonitor(libbox.InterfaceUpdateListener) error {
	return nil
}
func (*androidPlatform) CloseDefaultInterfaceMonitor(libbox.InterfaceUpdateListener) error {
	return nil
}
func (*androidPlatform) GetInterfaces() (libbox.NetworkInterfaceIterator, error) {
	interfaces, err := net.Interfaces()
	if err != nil {
		return nil, err
	}
	items := make([]*libbox.NetworkInterface, 0, len(interfaces))
	for _, item := range interfaces {
		addresses, addressErr := item.Addrs()
		if addressErr != nil {
			continue
		}
		prefixes := make([]string, 0, len(addresses))
		for _, address := range addresses {
			if _, network, err := net.ParseCIDR(address.String()); err == nil {
				prefixes = append(prefixes, network.String())
			}
		}
		items = append(items, &libbox.NetworkInterface{
			Index:     int32(item.Index),
			MTU:       int32(item.MTU),
			Name:      item.Name,
			Addresses: &androidStringIterator{values: prefixes},
			Flags:     int32(item.Flags),
			Type:      libbox.InterfaceTypeOther,
			DNSServer: &androidStringIterator{},
		})
	}
	return &androidNetworkInterfaceIterator{values: items}, nil
}
func (*androidPlatform) UnderNetworkExtension() bool                 { return false }
func (*androidPlatform) IncludeAllNetworks() bool                    { return false }
func (*androidPlatform) ReadWIFIState() *libbox.WIFIState            { return nil }
func (*androidPlatform) SystemCertificates() libbox.StringIterator   { return &androidStringIterator{} }
func (*androidPlatform) ClearDNSCache()                              {}
func (*androidPlatform) SendNotification(*libbox.Notification) error { return nil }

type androidStringIterator struct {
	values []string
}

func (i *androidStringIterator) Len() int32    { return int32(len(i.values)) }
func (i *androidStringIterator) HasNext() bool { return len(i.values) > 0 }
func (i *androidStringIterator) Next() string {
	if len(i.values) == 0 {
		return ""
	}
	value := i.values[0]
	i.values = i.values[1:]
	return value
}

type androidNetworkInterfaceIterator struct {
	values []*libbox.NetworkInterface
}

func (i *androidNetworkInterfaceIterator) HasNext() bool { return len(i.values) > 0 }
func (i *androidNetworkInterfaceIterator) Next() *libbox.NetworkInterface {
	if len(i.values) == 0 {
		return nil
	}
	value := i.values[0]
	i.values = i.values[1:]
	return value
}

func androidCError(err error) *C.char {
	if err == nil {
		setLastError(nil)
		return nil
	}
	setLastError(err)
	androidRuntime.Lock()
	androidRuntime.err = err
	androidRuntime.Unlock()
	return C.CString(err.Error())
}

//export kitty_singbox_android_start
func kitty_singbox_android_start(configContent *C.char, tunFD C.int, dataPath *C.char) *C.char {
	if configContent == nil || dataPath == nil {
		return androidCError(errors.New("missing Android VPN configuration"))
	}
	return androidCError(startAndroid(C.GoString(configContent), int32(tunFD), C.GoString(dataPath)))
}

//export kitty_singbox_android_stop
func kitty_singbox_android_stop() *C.char {
	return androidCError(stopAndroid())
}

//export kitty_singbox_android_traffic
func kitty_singbox_android_traffic() *C.char {
	payload, err := androidTraffic()
	if err != nil {
		return androidCError(err)
	}
	setLastError(nil)
	return C.CString(string(payload))
}

//export kitty_singbox_android_probe
func kitty_singbox_android_probe(configContent *C.char, nodeTagsJSON *C.char, probeURL *C.char, dataPath *C.char, resultOut **C.char) *C.char {
	if configContent == nil || nodeTagsJSON == nil || probeURL == nil || dataPath == nil || resultOut == nil {
		return androidCError(errors.New("missing Android probe configuration"))
	}
	var nodeTags []string
	if err := json.Unmarshal([]byte(C.GoString(nodeTagsJSON)), &nodeTags); err != nil {
		return androidCError(fmt.Errorf("decode Android probe tags: %w", err))
	}
	results, err := probeAndroid(
		C.GoString(configContent),
		nodeTags,
		C.GoString(probeURL),
		C.GoString(dataPath),
	)
	if err != nil {
		return androidCError(err)
	}
	payload, err := json.Marshal(results)
	if err != nil {
		return androidCError(fmt.Errorf("encode Android probe results: %w", err))
	}
	*resultOut = C.CString(string(payload))
	setLastError(nil)
	return nil
}
