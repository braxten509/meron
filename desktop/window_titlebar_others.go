//go:build !darwin

// The shared window startup code requests native titlebar
// alignment. Only the macOS implementation adjusts native
// titlebar controls. Other hosts satisfy the same call here.

package main

func alignNativeTitlebar() {}
