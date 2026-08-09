//go:build windows

package main

import "context"

func contextWithCacheFileOwner(ctx context.Context, _ string) context.Context {
	return ctx
}
