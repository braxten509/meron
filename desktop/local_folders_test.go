// Local account backups must be written outside Meron's
// active databases. The fixture checks that selecting a
// live database as a backup destination is rejected.

package main

import (
	"os"
	"path/filepath"
	"testing"
)

func TestLocalBackupRefusesLiveDatabases(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	for _, name := range []string{"archive.db", "archive.db-wal", "meron.db"} {
		if !isLiveDatabasePath(filepath.Join(appConfigDir(), name)) {
			t.Fatalf("%s accepted as a backup destination", name)
		}
	}
	if isLiveDatabasePath(filepath.Join(t.TempDir(), "archive.db")) {
		t.Fatal("an unrelated archive.db was refused")
	}

	// A link to the live archive is the archive, whatever it is called.
	live := filepath.Join(appConfigDir(), "archive.db")
	if err := os.MkdirAll(appConfigDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(live, []byte("db"), 0o600); err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	symlink := filepath.Join(dir, "backup.db")
	if err := os.Symlink(live, symlink); err != nil {
		t.Fatal(err)
	}
	hardlink := filepath.Join(dir, "other.db")
	if err := os.Link(live, hardlink); err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{symlink, hardlink} {
		if !isLiveDatabasePath(path) {
			t.Fatalf("%s links to the live archive but was accepted", path)
		}
	}
}
