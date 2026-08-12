package dologger

import (
	"testing"
)

// These tests require libdologger_core to be available at link time.
// Skip gracefully if the library is not installed.

func TestVersion(t *testing.T) {
	v := Version()
	if v == "" {
		t.Skip("DoLogger shared library not available (empty version string)")
	}
	t.Logf("DoLogger version: %s", v)
}

func TestNewLoggerDefault(t *testing.T) {
	log, err := NewLogger("")
	if err != nil {
		t.Skipf("DoLogger not available: %v", err)
	}
	defer log.Shutdown()

	if log.handle == nil {
		t.Fatal("handle is nil after successful init")
	}
}

func TestLogAllLevels(t *testing.T) {
	log, err := NewLogger("")
	if err != nil {
		t.Skipf("DoLogger not available: %v", err)
	}
	defer log.Shutdown()

	log.Trace("trace from Go test")
	log.Debug("debug from Go test")
	log.Info("info from Go test")
	log.Warn("warn from Go test")
	log.Error("error from Go test")
	log.Fatal("fatal from Go test")
	log.Audit("audit from Go test")
}

func TestStress(t *testing.T) {
	log, err := NewLogger("")
	if err != nil {
		t.Skipf("DoLogger not available: %v", err)
	}
	defer log.Shutdown()

	for i := 0; i < 1000; i++ {
		log.Info("Go stress test message")
	}
}

func TestShutdownTwice(t *testing.T) {
	log, err := NewLogger("")
	if err != nil {
		t.Skipf("DoLogger not available: %v", err)
	}

	log.Shutdown()
	// Second Shutdown must not crash (idempotent)
	log.Shutdown()
}
