// Package dologger provides a minimal Go wrapper around the DoLogger C ABI.
//
// Build Instructions
// ------------------
//
// 1. Build the DoLogger core shared library:
//
//      cd core/ && cargo build --release
//
// 2. Install the library where cgo's linker can find it.
//
//    Linux:
//      sudo cp target/release/libdologger_core.so /usr/local/lib/
//      sudo ldconfig
//
//    macOS:
//      cp target/release/libdologger_core.dylib /usr/local/lib/
//
//    Windows (MSYS2/MinGW):
//      cp target/release/dologger_core.dll /mingw64/bin/
//
// 3. Set CGO_LDFLAGS if the library is in a non-standard location:
//
//      export CGO_LDFLAGS="-L/path/to/core/target/release -ldologger_core"
//
// 4. Build and test:
//
//      go build ./...
//      go test -v ./...
//
// Usage
// -----
//
//      package main
//
//      import "github.com/dologger/adapters/go/dologger"
//
//      func main() {
//          log, err := dologger.NewLogger("")
//          if err != nil {
//              panic(err)
//          }
//          defer log.Shutdown()
//
//          log.Info("Hello from Go")
//          log.Warn("Disk usage at 85%")
//          log.Error("Connection refused")
//      }

package dologger

/*
#cgo LDFLAGS: -ldologger_core

#include <stdint.h>

// ---- Struct definitions matching the actual Rust FFI ABI ----
//
// These must match core/src/ffi.rs exactly.  The public header
// dologger_core.h documents a richer future interface; the actual
// exported dologger_log symbol uses the struct below.

typedef struct {
	int32_t  code;
	char     message[256];
	char     source_file[128];
	uint32_t source_line;
	uint8_t  _reserved[12];
} dologger_error_t;

typedef struct {
	uint8_t     level;
	const char *message;
	const char *source_file;
	uint32_t    source_line;
	uint8_t     _reserved[16];
} dologger_log_params_t;

typedef struct dologger_handle dologger_handle_t;

// ---- Exported symbols (match core/src/ffi.rs) ----

dologger_handle_t *dologger_init(const char *config_path, dologger_error_t *err);
int32_t            dologger_log(dologger_handle_t *handle, const dologger_log_params_t *params);
void               dologger_shutdown(dologger_handle_t *handle);
int32_t            dologger_get_last_error(const dologger_handle_t *handle, dologger_error_t *err);
const char        *dologger_version(void);
*/
import "C"

import (
	"fmt"
	"os"
	"runtime"
	"unsafe"
)

// Log level constants matching the DoLogger C ABI.
const (
	Trace uint8 = iota
	Debug
	Info
	Warn
	Error
	Fatal
	Audit
)

// Logger wraps an opaque DoLogger engine handle.
//
// Create one instance per process with [NewLogger].  Call [Logger.Shutdown]
// before the process exits to flush all pending records and release
// resources.
type Logger struct {
	handle *C.dologger_handle_t
}

// NewLogger initializes the DoLogger engine.
//
// If configPath is empty, auto-discovery and hardcoded defaults are used.
// Returns an error if the shared library is not found, the config is
// malformed, or the engine fails to initialize.
func NewLogger(configPath string) (*Logger, error) {
	var cConfig *C.char
	if configPath != "" {
		cConfig = C.CString(configPath)
		defer C.free(unsafe.Pointer(cConfig))
	}

	var err C.dologger_error_t
	handle := C.dologger_init(cConfig, &err)

	if handle == nil {
		code := int32(err.code)
		msg := C.GoString(&err.message[0])
		return nil, fmt.Errorf("dologger_init failed (code=%d): %s", code, msg)
	}

	l := &Logger{handle: handle}
	runtime.SetFinalizer(l, func(l *Logger) {
		if l.handle != nil {
			C.dologger_shutdown(l.handle)
			l.handle = nil
		}
	})
	return l, nil
}

// Trace logs a message at TRACE level.
func (l *Logger) Trace(msg string) {
	l.log(Trace, msg)
}

// Debug logs a message at DEBUG level.
func (l *Logger) Debug(msg string) {
	l.log(Debug, msg)
}

// Info logs a message at INFO level.
func (l *Logger) Info(msg string) {
	l.log(Info, msg)
}

// Warn logs a message at WARN level.
func (l *Logger) Warn(msg string) {
	l.log(Warn, msg)
}

// Error logs a message at ERROR level.
func (l *Logger) Error(msg string) {
	l.log(Error, msg)
}

// Fatal logs a message at FATAL level.
func (l *Logger) Fatal(msg string) {
	l.log(Fatal, msg)
}

// Audit logs a message at AUDIT level (non-repudiable, WORM, signed).
func (l *Logger) Audit(msg string) {
	l.log(Audit, msg)
}

// log submits a record at the given level through the C ABI.
func (l *Logger) log(level uint8, msg string) {
	if l.handle == nil {
		return
	}

	cMsg := C.CString(msg)
	defer C.free(unsafe.Pointer(cMsg))

	params := C.dologger_log_params_t{
		level:   C.uint8_t(level),
		message: cMsg,
	}
	rc := C.dologger_log(l.handle, &params)
	if rc != 0 {
		var err C.dologger_error_t
		C.dologger_get_last_error(l.handle, &err)
		errMsg := C.GoString(&err.message[0])
		fmt.Fprintf(
			os.Stderr,
			"[dologger] log dropped (code=%d): %s\n",
			int32(rc),
			errMsg,
		)
	}
}

// Shutdown gracefully shuts down the engine, draining the pipeline,
// flushing all sinks, and freeing resources.
func (l *Logger) Shutdown() {
	if l.handle != nil {
		C.dologger_shutdown(l.handle)
		l.handle = nil
	}
}

// Version returns the DoLogger core library version string.
func Version() string {
	return C.GoString(C.dologger_version())
}
