// Exercises Linux update replacement using temporary binaries and archives.
// Cases cover AppImages, tarballs, missing executables and managed channels.
// The assertions inspect the target written by the installer.

package main

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func writeTarGz(t *testing.T, path string, entries map[string]string) {
	t.Helper()
	var buffer bytes.Buffer
	gz := gzip.NewWriter(&buffer)
	tw := tar.NewWriter(gz)
	for name, content := range entries {
		header := &tar.Header{Name: name, Mode: 0o755, Size: int64(len(content)), Typeflag: tar.TypeReg}
		if err := tw.WriteHeader(header); err != nil {
			t.Fatal(err)
		}
		if _, err := tw.Write([]byte(content)); err != nil {
			t.Fatal(err)
		}
	}
	if err := tw.Close(); err != nil {
		t.Fatal(err)
	}
	if err := gz.Close(); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, buffer.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

func TestReplaceTargetAppImage(t *testing.T) {
	directory := t.TempDir()
	target := filepath.Join(directory, "Meron.AppImage")
	if err := os.WriteFile(target, []byte("old image"), 0o755); err != nil {
		t.Fatal(err)
	}
	payload := filepath.Join(t.TempDir(), "meron-linux-amd64.AppImage")
	if err := os.WriteFile(payload, []byte("new image"), 0o644); err != nil {
		t.Fatal(err)
	}

	if err := replaceTarget(updateChannel{Kind: channelAppImage, Target: target}, payload); err != nil {
		t.Fatalf("replaceTarget: %v", err)
	}

	got, err := os.ReadFile(target)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != "new image" {
		t.Fatalf("target contents = %q, want %q", got, "new image")
	}
	// The AppImage has to stay executable or the desktop entry breaks.
	info, err := os.Stat(target)
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm()&0o111 == 0 {
		t.Fatalf("target mode = %v, want the executable bit set", info.Mode().Perm())
	}
	assertNoLeftovers(t, directory, "Meron.AppImage")
}

func TestReplaceTargetTarball(t *testing.T) {
	directory := t.TempDir()
	target := filepath.Join(directory, "meron")
	if err := os.WriteFile(target, []byte("old binary"), 0o755); err != nil {
		t.Fatal(err)
	}
	payload := filepath.Join(t.TempDir(), "meron-linux-amd64.tar.gz")
	writeTarGz(t, payload, map[string]string{"meron": "new binary"})

	if err := replaceTarget(updateChannel{Kind: channelTarball, Target: target}, payload); err != nil {
		t.Fatalf("replaceTarget: %v", err)
	}
	got, err := os.ReadFile(target)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != "new binary" {
		t.Fatalf("target contents = %q, want %q", got, "new binary")
	}
	assertNoLeftovers(t, directory, "meron")
}

// A tarball without the binary must leave the installed copy untouched rather
// than replacing it with nothing.
func TestReplaceTargetTarballMissingBinary(t *testing.T) {
	directory := t.TempDir()
	target := filepath.Join(directory, "meron")
	if err := os.WriteFile(target, []byte("old binary"), 0o755); err != nil {
		t.Fatal(err)
	}
	payload := filepath.Join(t.TempDir(), "meron-linux-amd64.tar.gz")
	writeTarGz(t, payload, map[string]string{"README": "nope"})

	err := replaceTarget(updateChannel{Kind: channelTarball, Target: target}, payload)
	if err == nil {
		t.Fatal("expected an error")
	}
	if !strings.Contains(err.Error(), "not found") {
		t.Fatalf("error = %v, want a not-found error", err)
	}
	got, _ := os.ReadFile(target)
	if string(got) != "old binary" {
		t.Fatalf("target was modified: %q", got)
	}
	assertNoLeftovers(t, directory, "meron")
}

func TestReplaceTargetRejectsManagedChannel(t *testing.T) {
	if err := replaceTarget(updateChannel{Kind: channelSnap, Managed: true}, "/dev/null"); err == nil {
		t.Fatal("expected an error for a store-managed channel")
	}
}

// A read-only install dir (a root-owned /opt) must fail before anything moves.
func TestReplaceTargetUnwritableDirectory(t *testing.T) {
	if os.Geteuid() == 0 {
		t.Skip("root can write to a read-only directory")
	}
	directory := t.TempDir()
	target := filepath.Join(directory, "meron")
	if err := os.WriteFile(target, []byte("old binary"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(directory, 0o555); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.Chmod(directory, 0o755) })

	payload := filepath.Join(t.TempDir(), "meron-linux-amd64.AppImage")
	if err := os.WriteFile(payload, []byte("new image"), 0o644); err != nil {
		t.Fatal(err)
	}
	err := replaceTarget(updateChannel{Kind: channelAppImage, Target: target}, payload)
	if err == nil {
		t.Fatal("expected an error")
	}
	if !strings.Contains(err.Error(), "not writable") {
		t.Fatalf("error = %v, want a not-writable error", err)
	}
}

// Every failure path has to clean up its staging file, or the install dir fills
// with .meron-update-* debris.
func assertNoLeftovers(t *testing.T, directory string, expected ...string) {
	t.Helper()
	entries, err := os.ReadDir(directory)
	if err != nil {
		t.Fatal(err)
	}
	allowed := map[string]bool{}
	for _, name := range expected {
		allowed[name] = true
	}
	for _, entry := range entries {
		if !allowed[entry.Name()] {
			t.Errorf("unexpected leftover file %q in the install dir", entry.Name())
		}
	}
}
