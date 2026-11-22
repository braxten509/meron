// Exercises single-message and archive exports using a fake message fetcher.
// Tests inspect the written EML files and ZIP entries, including partial success.
// Cancellation and empty results must preserve an existing destination file.

package main

import (
	"archive/zip"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

// fakeEmlFetcher serves canned messages by UID; a UID with no message is
// "expunged" and left out of the saved list.
func fakeEmlFetcher(messages map[uint32]string, failFolder string) emlBatchFetcher {
	return func(source emlExportSource, uids []uint32, dir string) ([]uint32, error) {
		if source.Folder == failFolder {
			return nil, errors.New("offline")
		}
		var saved []uint32
		for _, uid := range uids {
			raw, ok := messages[uid]
			if !ok {
				continue
			}
			if err := os.WriteFile(filepath.Join(dir, fmt.Sprintf("%d.eml", uid)), []byte(raw), 0o600); err != nil {
				return nil, err
			}
			saved = append(saved, uid)
		}
		return saved, nil
	}
}

func zipEntries(t *testing.T, path string) map[string]string {
	t.Helper()
	reader, err := zip.OpenReader(path)
	if err != nil {
		t.Fatalf("open zip: %v", err)
	}
	defer reader.Close()
	entries := map[string]string{}
	for _, file := range reader.File {
		rc, err := file.Open()
		if err != nil {
			t.Fatalf("open %s: %v", file.Name, err)
		}
		data, _ := io.ReadAll(rc)
		rc.Close()
		entries[file.Name] = string(data)
	}
	return entries
}

func TestRunEmlExportZipsMessagesAndSkipsFailures(t *testing.T) {
	report := "Date: Sun, 04 Oct 2026 10:00:00 +0000\r\nSubject: Report\r\n\r\none"
	again := "Date: Sun, 04 Oct 2026 11:00:00 +0000\r\nSubject: =?UTF-8?B?UmVwb3J0?=\r\n\r\ntwo"
	undated := "Subject: a/b\r\n\r\nthree"
	sources := []emlExportSource{
		{Account: "acc", Folder: "INBOX", UIDs: []uint32{1, 2, 3, 4}},
		{Account: "acc", Folder: "Broken", UIDs: []uint32{9}},
	}
	dest := filepath.Join(t.TempDir(), "out.zip")
	var progress [][2]int
	result, err := runEmlExport(context.Background(), sources, dest,
		fakeEmlFetcher(map[uint32]string{1: report, 2: again, 3: undated}, "Broken"),
		func(done, failed int) { progress = append(progress, [2]int{done, failed}) })
	if err != nil {
		t.Fatalf("runEmlExport: %v", err)
	}
	if want := (emlExportResult{Exported: 3, Failed: 2}); result != want {
		t.Fatalf("result = %+v, want %+v", result, want)
	}
	if want := [][2]int{{3, 1}, {3, 2}}; !reflect.DeepEqual(progress, want) {
		t.Fatalf("progress = %v, want %v", progress, want)
	}
	want := map[string]string{
		"2026-10-04_Report.eml":   report,
		"2026-10-04_Report_2.eml": again,
		"ab.eml":                  undated,
	}
	if got := zipEntries(t, dest); !reflect.DeepEqual(got, want) {
		t.Fatalf("zip entries = %#v, want %#v", got, want)
	}
}

func TestRunEmlExportWritesALoneMessageAsIs(t *testing.T) {
	raw := "Subject: Solo\r\n\r\nbody"
	dest := filepath.Join(t.TempDir(), "Solo.eml")
	result, err := runEmlExport(context.Background(),
		[]emlExportSource{{Account: "acc", Folder: "INBOX", UIDs: []uint32{7}}}, dest,
		fakeEmlFetcher(map[uint32]string{7: raw}, ""), func(int, int) {})
	if err != nil || result.Exported != 1 {
		t.Fatalf("result = %+v, err = %v", result, err)
	}
	if got, _ := os.ReadFile(dest); string(got) != raw {
		t.Fatalf("saved message = %q, want %q", got, raw)
	}
}

// A cancelled or failed export must not touch what is already at dest, nor
// leave a staging file beside it.
func TestRunEmlExportKeepsAnExistingFileWhenCancelledOrEmpty(t *testing.T) {
	sources := []emlExportSource{{Account: "acc", Folder: "INBOX", UIDs: []uint32{1, 2}}}
	dir := t.TempDir()
	dest := filepath.Join(dir, "out.zip")
	if err := os.WriteFile(dest, []byte("previous export"), 0o600); err != nil {
		t.Fatal(err)
	}
	assertUntouched := func(when string) {
		t.Helper()
		if got, _ := os.ReadFile(dest); string(got) != "previous export" {
			t.Fatalf("%s export changed dest to %q", when, got)
		}
		if entries, _ := os.ReadDir(dir); len(entries) != 1 {
			t.Fatalf("%s export left %d files in the destination directory, want 1", when, len(entries))
		}
	}

	cancelled, cancel := context.WithCancel(context.Background())
	cancel()
	result, err := runEmlExport(cancelled, sources, dest, fakeEmlFetcher(nil, ""), func(int, int) {})
	if err != nil || !result.Cancelled {
		t.Fatalf("result = %+v, err = %v, want a cancelled result", result, err)
	}
	assertUntouched("cancelled")

	if _, err := runEmlExport(context.Background(), sources, dest, fakeEmlFetcher(nil, "INBOX"), func(int, int) {}); err == nil || err.Error() != "offline" {
		t.Fatalf("err = %v, want the fetch error", err)
	}
	assertUntouched("failed")
}

func TestRunEmlExportHonoursCancelDuringTheFinalFetch(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	inner := fakeEmlFetcher(map[uint32]string{1: "Subject: a\r\n\r\nx", 2: "Subject: b\r\n\r\ny"}, "")
	fetch := func(source emlExportSource, uids []uint32, dir string) ([]uint32, error) {
		cancel()
		return inner(source, uids, dir)
	}
	dest := filepath.Join(t.TempDir(), "out.zip")
	result, err := runEmlExport(ctx, []emlExportSource{{Account: "acc", Folder: "INBOX", UIDs: []uint32{1, 2}}}, dest, fetch, func(int, int) {})
	if err != nil || !result.Cancelled {
		t.Fatalf("result = %+v, err = %v, want a cancelled result", result, err)
	}
	if _, statErr := os.Stat(dest); !os.IsNotExist(statErr) {
		t.Fatalf("cancelled export wrote %s", dest)
	}
}

func TestRunEmlExportHonoursCancelAfterTheFinalBatchIsWritten(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	dest := filepath.Join(t.TempDir(), "out.zip")
	if err := os.WriteFile(dest, []byte("previous export"), 0o600); err != nil {
		t.Fatal(err)
	}
	result, err := runEmlExport(ctx, []emlExportSource{{Account: "acc", Folder: "INBOX", UIDs: []uint32{1, 2}}}, dest,
		fakeEmlFetcher(map[uint32]string{1: "Subject: a\r\n\r\nx", 2: "Subject: b\r\n\r\ny"}, ""),
		func(int, int) { cancel() })
	if err != nil || !result.Cancelled {
		t.Fatalf("result = %+v, err = %v, want a cancelled result", result, err)
	}
	if got, _ := os.ReadFile(dest); string(got) != "previous export" {
		t.Fatalf("cancelled export changed dest to %q", got)
	}
}

func TestRunEmlExportReplacesAnExistingFileOnSuccess(t *testing.T) {
	raw := "Subject: Solo\r\n\r\nbody"
	dest := filepath.Join(t.TempDir(), "Solo.eml")
	if err := os.WriteFile(dest, []byte("a much longer previous export"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := runEmlExport(context.Background(),
		[]emlExportSource{{Account: "acc", Folder: "INBOX", UIDs: []uint32{7}}}, dest,
		fakeEmlFetcher(map[uint32]string{7: raw}, ""), func(int, int) {}); err != nil {
		t.Fatalf("runEmlExport: %v", err)
	}
	if got, _ := os.ReadFile(dest); string(got) != raw {
		t.Fatalf("saved message = %q, want %q", got, raw)
	}
}

// Staging falls back to the temp dir when nothing but dest itself is writable.
func TestPublishEmlExportCopiesAcrossDirectories(t *testing.T) {
	staged := filepath.Join(t.TempDir(), "staged")
	dest := filepath.Join(t.TempDir(), "out.zip")
	if err := os.WriteFile(staged, []byte("new"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(dest, []byte("a longer previous export"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := publishEmlExport(staged, dest); err != nil {
		t.Fatalf("publishEmlExport: %v", err)
	}
	if got, _ := os.ReadFile(dest); string(got) != "new" {
		t.Fatalf("dest = %q, want %q", got, "new")
	}
}

func TestRunEmlExportStopsFetchingOnlyTheFailingAccount(t *testing.T) {
	uids := make([]uint32, 0, 60)
	for uid := uint32(1); uid <= 60; uid++ {
		uids = append(uids, uid)
	}
	calls := map[string]int{}
	healthy := fakeEmlFetcher(map[uint32]string{1: "Subject: a\r\n\r\nx", 2: "Subject: b\r\n\r\ny"}, "")
	fetch := func(source emlExportSource, batch []uint32, dir string) ([]uint32, error) {
		calls[source.Account]++
		if source.Account == "down" {
			return nil, errors.New("offline")
		}
		return healthy(source, batch, dir)
	}
	dest := filepath.Join(t.TempDir(), "out.zip")
	result, err := runEmlExport(context.Background(), []emlExportSource{
		{Account: "down", Folder: "INBOX", UIDs: uids},
		{Account: "up", Folder: "INBOX", UIDs: []uint32{1, 2}},
	}, dest, fetch, func(int, int) {})
	if err != nil {
		t.Fatalf("runEmlExport: %v", err)
	}
	if want := (emlExportResult{Exported: 2, Failed: 60}); result != want {
		t.Fatalf("result = %+v, want %+v", result, want)
	}
	if calls["down"] != emlExportMaxConsecutiveErrors || calls["up"] != 1 {
		t.Fatalf("fetch calls = %v, want %d for the failing account and 1 for the healthy one", calls, emlExportMaxConsecutiveErrors)
	}
	if got := zipEntries(t, dest); len(got) != 2 {
		t.Fatalf("zip entries = %#v, want the healthy account's two messages", got)
	}
}

func TestEmlExportSourcesMergesThreadsPerMailbox(t *testing.T) {
	app, writer := newMailHandlerTestApp(t,
		sidecarResponsePlan{Result: map[string]any{"uids": []any{float64(4), float64(5)}}},
		sidecarResponsePlan{Result: map[string]any{"uids": []any{float64(5), float64(8)}}},
	)
	sources, unresolved, err := app.emlExportSources(map[string]any{"threads": []any{
		map[string]any{"thread_id": formatImapThreadID("acc", "INBOX", "key"), "folder": "Work"},
		map[string]any{"thread_id": "acc#Work#8"},
		map[string]any{"thread_id": "rss-feed"},
	}})
	if err != nil {
		t.Fatalf("emlExportSources: %v", err)
	}
	want := []emlExportSource{{Account: "acc", Folder: "Work", UIDs: []uint32{4, 5, 8}}}
	if !reflect.DeepEqual(sources, want) || unresolved != 1 {
		t.Fatalf("sources = %+v, unresolved = %d, want %+v and 1", sources, unresolved, want)
	}
	assertCall(t, writer.calls[0], "messages.exportUids", map[string]any{"account": "acc", "folder": "Work", "thread_key": "key"})
	assertCall(t, writer.calls[1], "messages.exportUids", map[string]any{"account": "acc", "folder": "Work", "uid": float64(8)})
}

func TestEmlExportSourcesListsAWholeFolder(t *testing.T) {
	app, writer := newMailHandlerTestApp(t,
		sidecarResponsePlan{Result: map[string]any{"uids": []any{float64(1), float64(2)}}},
	)
	sources, _, err := app.emlExportSources(map[string]any{"account_id": "acc", "folder_id": "Archive"})
	if err != nil {
		t.Fatalf("emlExportSources: %v", err)
	}
	if want := []emlExportSource{{Account: "acc", Folder: "Archive", UIDs: []uint32{1, 2}}}; !reflect.DeepEqual(sources, want) {
		t.Fatalf("sources = %+v, want %+v", sources, want)
	}
	assertCall(t, writer.calls[0], "messages.exportUids", map[string]any{"account": "acc", "folder": "Archive", "all": true})
	if _, _, err := app.emlExportSources(map[string]any{"account_id": "unified", "folder_id": "inbox"}); err == nil {
		t.Fatal("expected the unified inbox to be refused")
	}
}
