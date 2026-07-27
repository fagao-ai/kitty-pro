package main

import (
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
