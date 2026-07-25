package main

import (
	"testing"

	boxlog "github.com/sagernet/sing-box/log"
)

func TestBridgeLogBufferReturnsIncrementalInfoLogs(t *testing.T) {
	var logs bridgeLogBuffer
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
