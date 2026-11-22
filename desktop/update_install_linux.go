// Applies a downloaded desktop update on Linux.
// The detected channel determines whether to replace an AppImage or a tarball binary.
// Archive handling locates the executable the running app will use next.

package main

import (
	"archive/tar"
	"compress/gzip"
	"fmt"
	"io"
	"os"
	"path/filepath"
)

// applyUpdate replaces the running AppImage (or the raw binary from the
// tarball) with the downloaded payload and queues a relaunch.
//
// The replacement is a rename within the target's own directory, so it is
// atomic and the running process keeps the old inode until it exits.
func applyUpdate(channel updateChannel, payload string) error {
	if operationError := replaceTarget(channel, payload); operationError != nil {
		return operationError
	}
	return scheduleRelaunch(channel.Target)
}

// replaceTarget is applyUpdate without the relaunch: everything that touches
// the filesystem, so it can be exercised on its own.
func replaceTarget(channel updateChannel, payload string) error {
	target := channel.Target
	if target == "" {
		return fmt.Errorf("update: unsupported channel %q", channel.Kind)
	}
	parent := filepath.Dir(target)
	if operationError := ensureWritableDir(parent); operationError != nil {
		return fmt.Errorf("update: %s is not writable — install the update manually: %w", parent, operationError)
	}

	staging := filepath.Join(parent, fmt.Sprintf(".meron-update-%d", os.Getpid()))
	_ = os.Remove(staging)

	var operationError error
	switch channel.Kind {
	case channelAppImage:
		// The payload is the new AppImage itself.
		operationError = copyFile(payload, staging, 0o755)
	case channelTarball:
		operationError = extractBinaryFromTarGz(payload, "meron", staging)
	default:
		return fmt.Errorf("update: unsupported channel %q", channel.Kind)
	}
	if operationError != nil {
		_ = os.Remove(staging)
		return operationError
	}

	if operationError := os.Chmod(staging, 0o755); operationError != nil {
		_ = os.Remove(staging)
		return operationError
	}
	if operationError := os.Rename(staging, target); operationError != nil {
		_ = os.Remove(staging)
		return fmt.Errorf("update: could not replace %s: %w", target, operationError)
	}
	return nil
}

// extractBinaryFromTarGz pulls a single entry out of the release tarball. The
// archive only ever holds the `meron` binary, but the name is matched on the
// basename so a leading directory in a future layout still works.
func extractBinaryFromTarGz(archive, name, dest string) error {
	file, operationError := os.Open(archive)
	if operationError != nil {
		return operationError
	}
	defer file.Close()
	gz, operationError := gzip.NewReader(file)
	if operationError != nil {
		return operationError
	}
	defer gz.Close()

	reader := tar.NewReader(gz)
	for {
		header, operationError := reader.Next()
		if operationError == io.EOF {
			return fmt.Errorf("update: %s not found in the downloaded archive", name)
		}
		if operationError != nil {
			return operationError
		}
		if header.Typeflag != tar.TypeReg || filepath.Base(header.Name) != name {
			continue
		}
		out, operationError := os.OpenFile(dest, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0o755)
		if operationError != nil {
			return operationError
		}
		if _, operationError := io.Copy(out, reader); operationError != nil {
			out.Close()
			return operationError
		}
		return out.Close()
	}
}
