package main

import (
	"net/http"
	"net/http/httptest"
	"os"
	"testing"

	boxlog "github.com/sagernet/sing-box/log"
)

func TestValidateRuleSetFileRejectsInvalidContent(t *testing.T) {
	file, err := os.CreateTemp(t.TempDir(), "invalid-*.srs")
	if err != nil {
		t.Fatalf("create invalid rule set: %v", err)
	}
	if _, err = file.WriteString("not a sing-box rule set"); err != nil {
		t.Fatalf("write invalid rule set: %v", err)
	}
	if err = file.Close(); err != nil {
		t.Fatalf("close invalid rule set: %v", err)
	}
	if err = validateRuleSetFile(file.Name()); err == nil {
		t.Fatal("invalid rule set unexpectedly passed validation")
	}
}

func TestBridgeLogBufferReturnsIncrementalInfoLogs(t *testing.T) {
	var logs bridgeLogBuffer
	logs.setEnabled(true)
	logs.WriteMessage(boxlog.LevelDebug, "DEBUG ignored")
	logs.WriteMessage(boxlog.LevelInfo, "\x1b[36mINFO\x1b[0m outbound/direct[direct]: outbound connection to example.cn:443")
	logs.recordConnectionRoute("example.cn:443", "direct", []string{"direct"})

	first := logs.snapshot(0)
	if first.NextCursor != 1 || len(first.Entries) != 1 {
		t.Fatalf("unexpected first batch: %+v", first)
	}
	if first.Entries[0].Message != "INFO outbound/direct[direct]: outbound connection to example.cn:443" {
		t.Fatalf("ANSI sequence was not removed: %q", first.Entries[0].Message)
	}
	if next := logs.snapshot(first.NextCursor); len(next.Entries) != 0 {
		t.Fatalf("incremental cursor returned duplicate entries: %+v", next)
	}
}

func TestBridgeLogBufferWaitsForConnectionChainBeforeAdvancingCursor(t *testing.T) {
	var logs bridgeLogBuffer
	logs.setEnabled(true)
	logs.WriteMessage(boxlog.LevelInfo, "outbound/anytls[node-tw]: outbound connection to chatgpt.com:443")

	pending := logs.snapshot(0)
	if pending.NextCursor != 0 || len(pending.Entries) != 0 {
		t.Fatalf("route log escaped before enrichment: %+v", pending)
	}

	logs.recordConnectionRoute("chatgpt.com:443", "node-tw", []string{"node-tw", "台湾节点", "AI节点"})
	batch := logs.snapshot(pending.NextCursor)
	if batch.NextCursor != 1 || len(batch.Entries) != 1 || batch.Entries[0].OutboundChain[0] != "AI节点" {
		t.Fatalf("enriched route log was not released: %+v", batch)
	}
}

func TestBridgeLogBufferAttachesConnectionChainAfterRouteLog(t *testing.T) {
	var logs bridgeLogBuffer
	logs.setEnabled(true)
	logs.WriteMessage(boxlog.LevelInfo, "outbound/anytls[node-tw]: outbound connection to chatgpt.com:443")
	logs.recordConnectionRoute("chatgpt.com:443", "node-tw", []string{"node-tw", "台湾节点", "AI节点"})

	batch := logs.snapshot(0)
	if len(batch.Entries) != 1 {
		t.Fatalf("unexpected log batch: %+v", batch)
	}
	want := []string{"AI节点", "台湾节点", "node-tw"}
	if got := batch.Entries[0].OutboundChain; len(got) != len(want) || got[0] != want[0] || got[1] != want[1] || got[2] != want[2] {
		t.Fatalf("unexpected outbound chain: %v", got)
	}
}

func TestBridgeLogBufferAttachesConnectionChainBeforeRouteLog(t *testing.T) {
	var logs bridgeLogBuffer
	logs.setEnabled(true)
	logs.recordConnectionRoute("[2001:db8::1]:443", "node-us", []string{"node-us", "美国节点", "漏网之鱼"})
	logs.WriteMessage(boxlog.LevelInfo, "outbound/vless[node-us]: outbound packet connection to [2001:db8::1]:443")

	batch := logs.snapshot(0)
	if len(batch.Entries) != 1 || batch.Entries[0].OutboundChain[0] != "漏网之鱼" {
		t.Fatalf("pending outbound chain was not attached: %+v", batch)
	}
}

func TestBridgeLogBufferResetsStaleCursor(t *testing.T) {
	var logs bridgeLogBuffer
	logs.setEnabled(true)
	logs.WriteMessage(boxlog.LevelWarn, "WARN first session")
	logs.reset()
	logs.WriteMessage(boxlog.LevelError, "ERROR second session")

	batch := logs.snapshot(99)
	if len(batch.Entries) != 1 || batch.Entries[0].Message != "ERROR second session" {
		t.Fatalf("stale cursor did not recover after reset: %+v", batch)
	}
}

func TestBridgeLogBufferKeepsOnlyNewestEntries(t *testing.T) {
	var logs bridgeLogBuffer
	logs.setEnabled(true)
	for index := 0; index < bridgeLogLimit+2; index++ {
		logs.WriteMessage(boxlog.LevelInfo, "INFO route")
	}

	batch := logs.snapshot(0)
	if len(batch.Entries) != bridgeLogLimit {
		t.Fatalf("unexpected retained log count: %d", len(batch.Entries))
	}
	if batch.Entries[0].Sequence != 3 || batch.Entries[len(batch.Entries)-1].Sequence != bridgeLogLimit+2 {
		t.Fatalf("log buffer did not retain the newest entries: %+v", batch)
	}
}

func TestBridgeLogBufferDoesNotCollectWhileDisabled(t *testing.T) {
	var logs bridgeLogBuffer
	logs.WriteMessage(boxlog.LevelInfo, "INFO ignored while disabled")

	batch := logs.snapshot(0)
	if batch.NextCursor != 0 || len(batch.Entries) != 0 {
		t.Fatalf("disabled log buffer retained entries: %+v", batch)
	}

	logs.setEnabled(true)
	logs.WriteMessage(boxlog.LevelInfo, "INFO collected while enabled")
	logs.setEnabled(false)
	logs.WriteMessage(boxlog.LevelInfo, "INFO ignored after pausing")

	batch = logs.snapshot(0)
	if batch.NextCursor != 1 || len(batch.Entries) != 1 {
		t.Fatalf("paused log buffer advanced its cursor: %+v", batch)
	}
}

func TestStartWithoutClashAPIDoesNotSubscribeLogs(t *testing.T) {
	config := `{
		"log":{"level":"error"},
		"inbounds":[],
		"outbounds":[{"type":"direct","tag":"direct"}],
		"route":{"auto_detect_interface":false,"final":"direct"}
	}`
	service, err := start(config)
	if err != nil {
		t.Fatalf("start config without Clash API: %v", err)
	}
	service.cancel()
	if err = service.box.Close(); err != nil {
		t.Fatalf("close config without Clash API: %v", err)
	}
}

func TestProbeReturnsMissingOutboundAsNodeError(t *testing.T) {
	config := `{
		"log":{"level":"error"},
		"inbounds":[],
		"outbounds":[{"type":"direct","tag":"direct"}],
		"route":{"auto_detect_interface":false,"final":"direct"}
	}`
	results, err := probe(config, []string{"missing"}, "https://www.gstatic.com/generate_204")
	if err != nil {
		t.Fatalf("probe config without Clash API: %v", err)
	}
	if len(results) != 1 || results[0].Tag != "missing" || results[0].Error == "" {
		t.Fatalf("unexpected missing outbound result: %+v", results)
	}
}

func TestProbeReturnsOneResultPerRequestedTag(t *testing.T) {
	config := `{
		"log": {"level": "error"},
		"inbounds": [],
		"outbounds": [{"type": "direct", "tag": "direct"}],
		"route": {"auto_detect_interface": false, "final": "direct"}
	}`
	tags := []string{"missing-a", "missing-b", "missing-c"}

	results, err := probe(config, tags, "https://www.gstatic.com/generate_204")
	if err != nil {
		t.Fatalf("probe failed: %v", err)
	}
	if len(results) != len(tags) {
		t.Fatalf("got %d results, want %d", len(results), len(tags))
	}
	for index, result := range results {
		if result.Tag != tags[index] {
			t.Fatalf("result %d tag = %q, want %q", index, result.Tag, tags[index])
		}
		if result.Error == "" {
			t.Fatalf("result %d should report the missing outbound", index)
		}
	}
}

func TestProbeUsesAnExistingCoreForSuccessfulOutboundTests(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(response http.ResponseWriter, _ *http.Request) {
		response.WriteHeader(http.StatusNoContent)
	}))
	defer server.Close()
	config := `{
		"log": {"level": "error"},
		"inbounds": [],
		"outbounds": [{"type": "direct", "tag": "direct"}],
		"route": {"auto_detect_interface": false, "final": "direct"}
	}`

	service, err := start(config)
	if err != nil {
		t.Fatalf("start failed: %v", err)
	}
	defer func() {
		service.cancel()
		_ = service.box.Close()
	}()
	result := service.probeOutbound("direct", server.URL)

	if result.Error != "" {
		t.Fatalf("probe failed: %s", result.Error)
	}
	if result.LatencyMS == nil {
		t.Fatal("probe should return a latency")
	}
}
