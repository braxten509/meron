//go:build !darwin && (!linux || !webkit2_41 || bindings)

// Selects frontend printing on platforms without a native mail-print adapter.
// The desktop command uses the same signature as the native implementations.
// Its return value tells the reader to perform the print operation.

package main

// Other platforms print directly in the frontend so JS errors reach its caller.
func printNativeMail(html string) (bool, string, error) {
	return false, "", nil
}
