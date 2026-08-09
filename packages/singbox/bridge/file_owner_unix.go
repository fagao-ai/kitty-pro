//go:build !windows

package main

import (
	"context"
	"os"
	"syscall"

	"github.com/sagernet/sing/service/filemanager"
)

// contextWithCacheFileOwner keeps cache files created by a privileged helper
// owned by the login user that prepared the file. The cache path is absolute,
// so the file manager only supplies ownership for files created during a
// corruption recovery.
func contextWithCacheFileOwner(ctx context.Context, path string) context.Context {
	uid, gid := cacheFileOwner(path)
	return filemanager.WithDefault(ctx, "", "", uid, gid)
}

func cacheFileOwner(path string) (int, int) {
	uid, gid := os.Getuid(), os.Getgid()
	if info, err := os.Stat(path); err == nil {
		if stat, ok := info.Sys().(*syscall.Stat_t); ok {
			uid, gid = int(stat.Uid), int(stat.Gid)
		}
	}
	return uid, gid
}
