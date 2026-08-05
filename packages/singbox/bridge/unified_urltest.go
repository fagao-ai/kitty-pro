package main

import (
	"context"
	"crypto/tls"
	"net"
	"net/http"
	"net/url"
	"time"

	"github.com/sagernet/sing-box/adapter"
	M "github.com/sagernet/sing/common/metadata"
	N "github.com/sagernet/sing/common/network"
	"github.com/sagernet/sing/common/ntp"
)

// unifiedURLTest matches Mihomo's unified-delay behavior by timing a second
// request on the connection warmed by the first request.
func unifiedURLTest(ctx context.Context, link string, detour N.Dialer) (uint16, error) {
	if link == "" {
		link = "https://www.gstatic.com/generate_204"
	}
	linkURL, err := url.Parse(link)
	if err != nil {
		return 0, err
	}
	port := linkURL.Port()
	if port == "" {
		switch linkURL.Scheme {
		case "http":
			port = "80"
		case "https":
			port = "443"
		}
	}

	start := time.Now()
	instance, err := detour.DialContext(
		ctx,
		"tcp",
		M.ParseSocksaddrHostPortStr(linkURL.Hostname(), port),
	)
	if err != nil {
		return 0, err
	}
	defer instance.Close()

	request, err := http.NewRequestWithContext(ctx, http.MethodHead, link, nil)
	if err != nil {
		return 0, err
	}
	client := http.Client{
		Transport: &http.Transport{
			DialContext: func(context.Context, string, string) (net.Conn, error) {
				return instance, nil
			},
			TLSClientConfig: &tls.Config{
				Time:    ntp.TimeFuncFromContext(ctx),
				RootCAs: adapter.RootPoolFromContext(ctx),
			},
		},
		CheckRedirect: func(*http.Request, []*http.Request) error {
			return http.ErrUseLastResponse
		},
	}
	defer client.CloseIdleConnections()

	response, err := client.Do(request)
	if err != nil {
		return 0, err
	}
	response.Body.Close()

	secondStart := time.Now()
	secondResponse, secondErr := client.Do(request)
	if secondErr == nil {
		secondResponse.Body.Close()
		start = secondStart
	}

	return uint16(time.Since(start) / time.Millisecond), nil
}
