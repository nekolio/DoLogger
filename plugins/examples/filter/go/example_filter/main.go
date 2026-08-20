// Example Filter plugin for DoLogger — Go implementation.
//
// This filter demonstrates the DoLogger C ABI plugin interface: it drops log
// records below a configurable minimum severity level.
//
// # Build (from this directory)
//
//	go build -buildmode=c-shared -o dologger-plugin-filter-go.so main.go
//
// # C ABI symbols exported
//
//	plugin_query(core_abi_version) → returns *PluginInfo
//	plugin_init(config)             → parses config, returns 0
//	plugin_shutdown()               → cleanup, returns 0
//
// The PluginInfo and VTable structs match the dologger_core.h definitions
// exactly, ensuring ABI compatibility.

package main

/*
#include <stdint.h>
#include <stdlib.h>

// ---------------------------------------------------------------------------
// dologger_plugin_info_t — must match dologger_core.h byte-for-byte.
// Field order: name, version, abi_version, phase, vtable.
// ---------------------------------------------------------------------------
typedef struct dologger_plugin_info {
	const char *name;
	uint32_t    version;
	uint32_t    abi_version;
	uint32_t    phase;
	void       *vtable;
} dologger_plugin_info_t;

// ---------------------------------------------------------------------------
// Filter VTable — matches dologger_filter_vtable_t in dologger_core.h.
// filter(rec, config):
//   Returns 0 to keep the record, non-zero to drop it.
// ---------------------------------------------------------------------------
typedef struct dologger_filter_vtable {
	int (*filter)(const void *rec, void *config);
} dologger_filter_vtable_t;

// ---------------------------------------------------------------------------
// go_filter_impl — the actual filter function, implemented in Go and
// exported to C via `//export`.  Declared here so the static vtable
// can reference it; the linker resolves the symbol at build time.
// ---------------------------------------------------------------------------
extern int go_filter_impl(const void *rec, void *config);

// ---------------------------------------------------------------------------
// Static VTable — initialized at compile time with the Go filter function.
// Accessible from Go code as C.go_filter_vtable.
// ---------------------------------------------------------------------------
static dologger_filter_vtable_t go_filter_vtable = {
	.filter = go_filter_impl
};
*/
import "C"

import (
	"encoding/json"
	"sync/atomic"
	"unsafe"
)

// ---------------------------------------------------------------------------
// Constants — must match dologger_core.h
// ---------------------------------------------------------------------------

const (
	levelTrace uint32 = 0
	levelDebug uint32 = 1
	levelInfo  uint32 = 2
	levelWarn  uint32 = 3
	levelError uint32 = 4
	levelFatal uint32 = 5
	levelAudit uint32 = 6

	phaseFilter    uint32 = 0x0002   // DO_LOG_PHASE_FILTER
	pluginVersion  uint32 = 0x000001 // 0.0.1 packed as major.minor.patch
	coreAbiVersion uint32 = 0x000001 // 0.0.1
)

// ---------------------------------------------------------------------------
// Plugin state
// ---------------------------------------------------------------------------

// minLevel is the minimum log level that passes the filter.
// Records with a level value >= minLevel are kept.
// Default: WARN (drops TRACE, DEBUG, INFO).
var minLevel atomic.Uint32

func init() {
	minLevel.Store(levelWarn)
}

// ---------------------------------------------------------------------------
// Plugin name — a null-terminated C string that lives for the lifetime of
// the shared library.
// ---------------------------------------------------------------------------

var pluginNameC *C.char

func init() {
	pluginNameC = C.CString("go-example-filter")
}

// ---------------------------------------------------------------------------
// PluginInfo — heap-allocated, returned by plugin_query().
//
// A production plugin typically uses a static PluginInfo.  We allocate here
// to demonstrate a pattern that works when vtable/data are initialized at
// runtime (e.g., after config parsing in more complex plugins).
// ---------------------------------------------------------------------------

func makePluginInfo() *C.dologger_plugin_info_t {
	info := (*C.dologger_plugin_info_t)(C.malloc(C.size_t(unsafe.Sizeof(C.dologger_plugin_info_t{}))))
	info.name = pluginNameC
	info.version = C.uint32_t(pluginVersion)
	info.abi_version = C.uint32_t(coreAbiVersion)
	info.phase = C.uint32_t(phaseFilter)
	info.vtable = unsafe.Pointer(&C.go_filter_vtable)
	return info
}

// ---------------------------------------------------------------------------
// C ABI: plugin_query
//
// Called once at load time. Returns a pointer to a dologger_plugin_info_t
// with the plugin identity and the Filter VTable.
// ---------------------------------------------------------------------------

//export plugin_query
func plugin_query(coreAbiVersion C.uint32_t) *C.dologger_plugin_info_t {
	// A production plugin should check core_abi_version compatibility here:
	//   if coreAbiVersion > CORE_ABI_VERSION { return nil }
	_ = coreAbiVersion
	return makePluginInfo()
}

// ---------------------------------------------------------------------------
// C ABI: plugin_init
//
// Receives a JSON config string:
//
//	{"min_level": 3}
//
// min_level: 0=TRACE, 1=DEBUG, 2=INFO, 3=WARN, 4=ERROR, 5=FATAL, 6=AUDIT.
// Omitted or null → keep default (WARN).
// ---------------------------------------------------------------------------

type pluginConfig struct {
	MinLevel *uint32 `json:"min_level"`
}

//export plugin_init
func plugin_init(config unsafe.Pointer) C.int {
	if config == nil {
		return 0 // use default min_level
	}

	configStr := C.GoString((*C.char)(config))
	if configStr == "" {
		return 0
	}

	var cfg pluginConfig
	if err := json.Unmarshal([]byte(configStr), &cfg); err != nil {
		// Invalid JSON — keep default, return OK (non-fatal).
		return 0
	}

	if cfg.MinLevel != nil {
		lvl := *cfg.MinLevel
		if lvl > levelAudit {
			lvl = levelAudit
		}
		minLevel.Store(lvl)
	}

	return 0
}

// ---------------------------------------------------------------------------
// C ABI: plugin_shutdown
//
// Called before library unload.  Resets state.  Returns 0 on success.
// ---------------------------------------------------------------------------

//export plugin_shutdown
func plugin_shutdown() C.int {
	// Reset to default so a subsequent init starts fresh.
	minLevel.Store(levelWarn)
	return 0
}

// ---------------------------------------------------------------------------
// Filter function — the actual filtering logic.
//
// This is the function pointed to by go_filter_vtable.filter.
// It is exported to C via `//export go_filter_impl`.
//
// C ABI filter contract:
//   Returns 0 to keep the record, non-zero to drop it.
//   MUST NOT perform I/O.
// ---------------------------------------------------------------------------

//export go_filter_impl
func go_filter_impl(rec unsafe.Pointer, config unsafe.Pointer) C.int {
	// config is passed as a pointer to the record's level (uint32).
	// In a real plugin the level would be read from the record handle
	// via the core's field access API; this simplified example accepts
	// it directly.
	if config == nil {
		return 0 // allow all if no level info
	}

	recordLevel := *(*uint32)(config)
	min := minLevel.Load()

	if recordLevel < min {
		return 1 // drop
	}
	return 0 // pass
}

// ---------------------------------------------------------------------------
// Required: main function (no-op for c-shared library)
// ---------------------------------------------------------------------------

func main() {}
