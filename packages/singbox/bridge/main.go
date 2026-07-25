// Package main builds a stable C ABI around the sing-box Go library.
package main

/*
#include <stdlib.h>
#include <stdint.h>
*/
import "C"

import (
	"context"
	stdjson "encoding/json"
	"fmt"
	"net/http"
	"sync"
	"time"
	"unsafe"

	box "github.com/sagernet/sing-box"
	CBox "github.com/sagernet/sing-box/constant"
	_ "github.com/sagernet/sing-box/experimental/clashapi"
	"github.com/sagernet/sing-box/include"
	"github.com/sagernet/sing-box/option"
	"github.com/sagernet/sing/common/json"
)

type instance struct {
	box              *box.Box
	cancel           context.CancelFunc
	trafficURL       string
	trafficAuthToken string
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
	service, err := box.New(box.Options{
		Context: ctx,
		Options: options,
	})
	if err != nil {
		cancel()
		return nil, err
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
	}, nil
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

//export kitty_singbox_start
func kitty_singbox_start(configContent *C.char) C.uint64_t {
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
func kitty_singbox_stop(handle C.uint64_t) C.int {
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

//export kitty_singbox_traffic
func kitty_singbox_traffic(handle C.uint64_t) *C.char {
	state.Lock()
	service, found := state.instances[uint64(handle)]
	state.Unlock()
	if !found {
		setLastError(&bridgeError{message: "sing-box instance is not running"})
		return nil
	}
	result, err := service.traffic()
	if err != nil {
		setLastError(err)
		return nil
	}
	setLastError(nil)
	return C.CString(string(result))
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
