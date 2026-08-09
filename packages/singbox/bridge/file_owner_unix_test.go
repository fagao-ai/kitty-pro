//go:build !windows

package main

import (
	"os"
	"syscall"
	"testing"
)

func TestCacheFileOwnerUsesPreparedFile(t *testing.T) {
	file, err := os.CreateTemp(t.TempDir(), "cache-*.db")
	if err != nil {
		t.Fatalf("create cache file: %v", err)
	}
	if err = file.Close(); err != nil {
		t.Fatalf("close cache file: %v", err)
	}
	info, err := os.Stat(file.Name())
	if err != nil {
		t.Fatalf("stat cache file: %v", err)
	}
	stat, ok := info.Sys().(*syscall.Stat_t)
	if !ok {
		t.Fatal("cache metadata does not expose unix ownership")
	}

	uid, gid := cacheFileOwner(file.Name())
	if uid != int(stat.Uid) || gid != int(stat.Gid) {
		t.Fatalf("unexpected cache owner: got %d:%d want %d:%d", uid, gid, stat.Uid, stat.Gid)
	}
}
