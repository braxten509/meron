//go:build integration

// Starts the isolated mail server used by desktop integration tests.
// The harness selects available ports and configures the container.
// Test helpers run commands against that fixture for the duration of each suite.

package main

// Test harness that runs a real Postfix/Dovecot mail server via Docker and the Rust
// sidecar so the integration tests can exercise the full IMAP/SMTP/store path.
// Build the sidecar first: `cargo build --manifest-path ../meron-core/Cargo.toml`,
// then run with `go test -tags integration .`.

import (
	"fmt"
	"io"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

const (
	mailServerImage = "ghcr.io/docker-mailserver/docker-mailserver:16.0.1"
	testPassword    = "testpass"
	mailHostname    = "mail.test"
)

// Loopback-only test fixture: plaintext IMAP and authenticated SMTP submission.
// DMS pre-creates Drafts, Sent, Trash and Junk; add Archive for the same roles
// used by the app. Filtering is disabled so synthetic messages stay unchanged.
const dovecotConfiguration = `auth_allow_cleartext = yes
ssl = no
namespace inbox {
    mailbox Archive {
        auto = subscribe
        special_use = \Archive
    }
}
`

type mailServer struct {
	container string
	imapPort  int
	smtpPort  int
}

func freePort(t *testing.T) int {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("freePort: %v", err)
	}
	defer listener.Close()
	return listener.Addr().(*net.TCPAddr).Port
}

func dockerBin(t *testing.T) string {
	t.Helper()
	bin, err := exec.LookPath("docker")
	if err != nil {
		t.Skip("docker not found on PATH; skipping mail server integration tests")
	}
	return bin
}

// runCmd returns the command's stdout only — `docker run` writes image pull
// progress to stderr, which must not pollute the captured container ID.
func runCmd(t *testing.T, name string, args ...string) string {
	t.Helper()
	var stderr strings.Builder
	cmd := exec.Command(name, args...)
	cmd.Stderr = &stderr
	out, err := cmd.Output()
	if err != nil {
		t.Fatalf("%s %s: %v\n%s%s", name, strings.Join(args, " "), err, out, stderr.String())
	}
	return strings.TrimSpace(string(out))
}

// startMailServer launches a throwaway Postfix/Dovecot container with two
// accounts (alice@mail.test, bob@mail.test). Each test owns its config and mail.
func startMailServer(t *testing.T) *mailServer {
	t.Helper()
	return startMailServerWithConfig(t, "")
}

func startMailServerWithConfig(t *testing.T, extraDovecot string) *mailServer {
	t.Helper()
	docker := dockerBin(t)
	config := t.TempDir()
	for name, contents := range map[string]string{
		"postfix-accounts.cf": "alice@" + mailHostname + "|{PLAIN}" + testPassword + "\n" +
			"bob@" + mailHostname + "|{PLAIN}" + testPassword + "\n",
		"dovecot.cf": dovecotConfiguration + extraDovecot,
		"postfix-master.cf": "submission/inet/smtpd_tls_security_level=none\n" +
			"submission/inet/smtpd_tls_auth_only=no\n",
	} {
		if err := os.WriteFile(filepath.Join(config, name), []byte(contents), 0o644); err != nil {
			t.Fatalf("write %s: %v", name, err)
		}
	}
	server := &mailServer{imapPort: freePort(t), smtpPort: freePort(t)}
	// The OS can hand back the same ephemeral port after it is released.
	for server.smtpPort == server.imapPort {
		server.smtpPort = freePort(t)
	}
	// :Z relabels the bind mount for SELinux hosts; harmless elsewhere.
	server.container = runCmd(t, docker, "run", "-d", "--rm",
		"--hostname", "mail."+mailHostname,
		"-v", config+":/tmp/docker-mailserver:Z",
		"-p", fmt.Sprintf("127.0.0.1:%d:143", server.imapPort),
		"-p", fmt.Sprintf("127.0.0.1:%d:587", server.smtpPort),
		"-e", "SSL_TYPE=",
		"-e", "ENABLE_AMAVIS=0",
		"-e", "ENABLE_SPAMASSASSIN=0",
		"-e", "ENABLE_CLAMAV=0",
		"-e", "ENABLE_RSPAMD=0",
		"-e", "ENABLE_OPENDKIM=0",
		"-e", "ENABLE_OPENDMARC=0",
		"-e", "ENABLE_POLICYD_SPF=0",
		"-e", "ENABLE_FAIL2BAN=0",
		"-e", "ENABLE_DNSBL=0",
		"-e", "ENABLE_MTA_STS=0",
		"-e", "ENABLE_UPDATE_CHECK=0",
		"-e", "LOG_LEVEL=warn",
		mailServerImage)
	t.Cleanup(func() {
		logs, err := exec.Command(docker, "logs", server.container).CombinedOutput()
		if err == nil {
			t.Logf("mail server logs:\n%s", logs)
		} else {
			t.Logf("failed to get mail server logs: %v", err)
		}
		out, err := exec.Command(docker, "stop", "-t", "1", server.container).CombinedOutput()
		if err != nil {
			t.Logf("docker stop %s: %v\n%s", server.container, err, out)
		}
	})
	waitForIMAPGreeting(t, server.imapPort, docker, server.container)
	waitForSMTPGreeting(t, server.smtpPort, docker, server.container)
	return server
}

func waitForSMTPGreeting(t *testing.T, port int, docker, container string) {
	t.Helper()
	waitForServerGreeting(t, port, "220 ", docker, container)
}

func waitForIMAPGreeting(t *testing.T, port int, docker, container string) {
	t.Helper()
	waitForServerGreeting(t, port, "* OK", docker, container)
}

func waitForServerGreeting(t *testing.T, port int, prefix, docker, container string) {
	t.Helper()
	deadline := time.Now().Add(60 * time.Second)
	for time.Now().Before(deadline) {
		conn, err := net.DialTimeout("tcp", fmt.Sprintf("127.0.0.1:%d", port), time.Second)
		if err == nil {
			conn.SetReadDeadline(time.Now().Add(2 * time.Second))
			buf := make([]byte, 256)
			n, _ := conn.Read(buf)
			conn.Close()
			if strings.HasPrefix(string(buf[:n]), prefix) {
				return
			}
		}
		time.Sleep(300 * time.Millisecond)
	}
	logs, _ := exec.Command(docker, "logs", container).CombinedOutput()
	t.Fatalf("mail server did not become ready on port %d\ncontainer logs:\n%s", port, logs)
}

// restartMailServer stops and starts the server container, standing in for a mail
// server that goes away mid-session (restart, deploy, transient outage). The
// mailbox state lives in the container filesystem, so it survives.
func restartMailServer(t *testing.T, server *mailServer) {
	t.Helper()
	docker := dockerBin(t)
	runCmd(t, docker, "restart", "-t", "3", server.container)
	waitForIMAPGreeting(t, server.imapPort, docker, server.container)
	waitForSMTPGreeting(t, server.smtpPort, docker, server.container)
}

// startSidecar launches the Rust core against a throwaway profile dir, and records the events it pushes.
//
// The sidecar resolves its DB/media paths from XDG dirs (sidecarEnv), so pointing XDG_* and HOME at a
// temp dir isolates it; MERON_KEYRING=off keeps the test run out of the OS keychain.
//
// The profile and the env pointing at it are scoped to the `t` passed in, so calling this from a
// subtest yields a *second*, empty profile against the same server — a cold store whose contents can
// only have come from IMAP.
func startSidecar(t *testing.T) (*Sidecar, *eventLog) {
	t.Helper()
	bin := "../meron-core/target/debug/meron-core"
	if _, err := os.Stat(bin); err != nil {
		t.Fatalf("sidecar binary missing at %s — run `cargo build --manifest-path ../meron-core/Cargo.toml` first", bin)
	}

	profile := t.TempDir()
	t.Setenv("HOME", profile)
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(profile, "config"))
	t.Setenv("XDG_CACHE_HOME", filepath.Join(profile, "cache"))
	t.Setenv("MERON_KEYRING", "off")

	sidecar := NewSidecar(bin, os.Stderr)
	events := &eventLog{}
	// Must be installed before Start: the read loop is what dispatches events.
	sidecar.onEvent = events.record
	if err := sidecar.Start(nil); err != nil {
		t.Fatalf("start sidecar: %v", err)
	}
	t.Cleanup(sidecar.Close)
	return sidecar, events
}

// eventLog collects the push events the sidecar emits (mail.newMessages,
// mail.synced, …) so tests can assert on the push path — IDLE has no request to
// hang an assertion off.
type eventLog struct {
	mu     sync.Mutex
	events []sidecarEvent
}

type sidecarEvent struct {
	name   string
	detail map[string]any
}

func (eventLogState *eventLog) record(name string, detail any) {
	object, _ := detail.(map[string]any)
	eventLogState.mu.Lock()
	defer eventLogState.mu.Unlock()
	eventLogState.events = append(eventLogState.events, sidecarEvent{name: name, detail: object})
}

// match returns the events so far satisfying predicate.
func (eventLogState *eventLog) match(predicate func(sidecarEvent) bool) []sidecarEvent {
	eventLogState.mu.Lock()
	defer eventLogState.mu.Unlock()
	var out []sidecarEvent
	for _, event := range eventLogState.events {
		if predicate(event) {
			out = append(out, event)
		}
	}
	return out
}

// count returns how many recorded events satisfy predicate. Tests compare it
// across an action rather than asserting on an absolute number: watchers emit a
// catch-up event when they start, so only the delta is meaningful.
func (events *eventLog) count(predicate func(sidecarEvent) bool) int {
	return len(events.match(predicate))
}

// connectAccount registers an account on the sidecar against the local mail server.
func connectAccount(t *testing.T, sidecar *Sidecar, server *mailServer, id, user string) {
	t.Helper()
	_, err := sidecar.Call("account.connect", map[string]any{
		"account":   id,
		"email":     user,
		"host":      "127.0.0.1",
		"port":      server.imapPort,
		"tls":       false,
		"smtp_host": "127.0.0.1",
		"smtp_port": server.smtpPort,
		"smtp_tls":  false,
		"user":      user,
		"password":  testPassword,
		"validate":  true,
	})
	if err != nil {
		t.Fatalf("account.connect %s: %v", id, err)
	}
}

// callMap invokes a sidecar method and asserts the result is a JSON object.
func callMap(t *testing.T, sidecar *Sidecar, method string, params map[string]any) map[string]any {
	t.Helper()
	result, err := sidecar.Call(method, params)
	if err != nil {
		t.Fatalf("%s: %v", method, err)
	}
	object, ok := result.(map[string]any)
	if !ok {
		t.Fatalf("%s: expected object result, got %T", method, result)
	}
	return object
}

// imapProxy lets integration tests interrupt an existing IMAP connection.
//
// imapProxy forwards IMAP connections to the mail server container so a test can cut the client's live
// sockets on demand, standing in for a pooled connection the server dropped without telling the client.
//
// Cutting it here rather than by restarting or unplugging the container keeps the server itself reachable —
// and Docker's host-side port forwarding survives a container restart anyway, so the socket the client
// holds would stay usable.
type imapProxy struct {
	listener net.Listener
	mu       sync.Mutex
	conns    []net.Conn
}

// startIMAPProxy listens on a free localhost port and forwards to targetPort.
func startIMAPProxy(t *testing.T, targetPort int) *imapProxy {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen for imap proxy: %v", err)
	}
	proxy := &imapProxy{listener: listener}
	t.Cleanup(func() {
		listener.Close()
		proxy.sever()
	})
	go func() {
		for {
			client, err := listener.Accept()
			if err != nil {
				return
			}
			upstream, err := net.DialTimeout("tcp", fmt.Sprintf("127.0.0.1:%d", targetPort), 5*time.Second)
			if err != nil {
				client.Close()
				continue
			}
			proxy.track(client, upstream)
			go func() {
				defer client.Close()
				defer upstream.Close()
				io.Copy(upstream, client)
			}()
			go func() {
				defer client.Close()
				defer upstream.Close()
				io.Copy(client, upstream)
			}()
		}
	}()
	return proxy
}

func (proxy *imapProxy) port() int {
	return proxy.listener.Addr().(*net.TCPAddr).Port
}

func (proxy *imapProxy) track(conns ...net.Conn) {
	proxy.mu.Lock()
	defer proxy.mu.Unlock()
	proxy.conns = append(proxy.conns, conns...)
}

// sever closes every connection opened so far. Connections made afterwards are
// proxied normally, so a client that reconnects recovers.
func (proxy *imapProxy) sever() {
	proxy.mu.Lock()
	conns := proxy.conns
	proxy.conns = nil
	proxy.mu.Unlock()
	for _, conn := range conns {
		// RST rather than FIN: a half-closed socket still accepts writes, so
		// the client would only notice several commands in. A reset surfaces
		// on its very next command, like a server that dropped the session.
		if tcp, ok := conn.(*net.TCPConn); ok {
			tcp.SetLinger(0)
		}
		conn.Close()
	}
}
