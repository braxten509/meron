//go:build darwin

// Prints an open message using the native macOS print dialog.
// The desktop reader supplies the rendered message HTML.
// The platform adapter returns the print result to the reader command.

package main

/*
#cgo CFLAGS: -x objective-c
#cgo LDFLAGS: -framework AppKit -framework WebKit
#include <stdlib.h>
int printMailDocument(const char *html);
*/
import "C"

import (
	"fmt"
	"unsafe"
)

func printNativeMail(html string) (bool, string, error) {
	document := C.CString(html)
	defer C.free(unsafe.Pointer(document))
	result := C.printMailDocument(document)
	if result < 0 {
		return false, "print document unavailable", nil
	}
	if result == 0 {
		return false, "", fmt.Errorf("could not present mail print dialog")
	}
	return true, "", nil
}
