/**
 * dologger_shm.h — Shared Memory Sink consumer API.
 *
 * This header defines the shared memory layout used by sink_shm.
 * External consumer processes (monitoring agents, TUI dashboards, etc.)
 * use these structures to attach to the shared memory ring buffer
 * and read SIF-formatted records with bounded validation.
 *
 * # Quick start for consumers
 *
 * 1. Open the shared memory object:
 *    - Linux/macOS: `shm_open(name, O_RDONLY, 0)` + `mmap()`
 *    - Windows: `OpenFileMappingW(FILE_MAP_READ, FALSE, name)` + `MapViewOfFile()`
 *
 * 2. Validate the header:
 *    ```c
 *    dologger_shm_header_t *hdr = (dologger_shm_header_t*)ptr;
 *    if (hdr->magic != DOLOGGER_SHM_MAGIC) { ... error ... }
 *    if (hdr->version != DOLOGGER_SHM_VERSION) { ... error ... }
 *    ```
 *
 * 3. Read records in a loop:
 *    ```c
 *    uint64_t consumer_seq = atomic_load(&hdr->consumer_seq);
 *    uint64_t producer_seq = atomic_load(&hdr->producer_seq);
 *    while (producer_seq != consumer_seq) {
 *        uint32_t slot_idx = consumer_seq % hdr->slot_count;
 *        void *slot = ptr + DOLOGGER_SHM_HEADER_SIZE + slot_idx * hdr->slot_size_bytes;
 *        uint32_t record_len = *(uint32_t*)slot;
 *        void *sif_data = slot + 4;     // 4B LE length prefix
 *        // validate sif_data[0..record_len] as SIF before reading it
 *        consumer_seq++;
 *        CAS(&hdr->consumer_seq, consumer_seq - 1, consumer_seq);
 *    }
 *    ```
 *
 * 4. Cleanup:
 *    - `munmap(ptr, size)` / `UnmapViewOfFile(ptr)`
 *    - `close(fd)` / `CloseHandle(handle)`
 *    - Consumers do NOT call `shm_unlink` — the producer owns cleanup
 */

#ifndef DOLOGGER_SHM_H
#define DOLOGGER_SHM_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* -------------------------------------------------------------------------
 * Magic & version
 * ------------------------------------------------------------------------- */

/** Magic value for shared memory validation (ASCII "DLOG"). */
#define DOLOGGER_SHM_MAGIC   0x474F4C44u

/** Current shared memory layout version. */
#define DOLOGGER_SHM_VERSION 1u

/** Size of the shared memory header in bytes. */
#define DOLOGGER_SHM_HEADER_SIZE 64

/* -------------------------------------------------------------------------
 * Flags
 * ------------------------------------------------------------------------- */

/** Producer (DoLogger) is alive and actively writing. */
#define DOLOGGER_SHM_FLAG_PRODUCER_ALIVE    0x00000001u
/** Producer has shut down cleanly (dologger_shutdown called). */
#define DOLOGGER_SHM_FLAG_PRODUCER_DEAD     0x00000002u
/** Buffer has overflowed — some records were dropped. */
#define DOLOGGER_SHM_FLAG_BUFFER_OVERFLOW   0x00000004u

/* -------------------------------------------------------------------------
 * Shared memory header layout
 * ------------------------------------------------------------------------- */

/**
 * Header at the start of the shared memory region.
 *
 * This structure is 64 bytes and cache-line aligned (alignas(64)).
 * ALL fields must be accessed atomically using the platform's atomic
 * intrinsics (C11 stdatomic.h, C++11 std::atomic, or equivalent).
 */
typedef struct {
    /** Total buffer size in bytes (header + all slots). */
    uint64_t buffer_size_bytes;

    /**
     * Consumer sequence — next slot to read.
     * Advanced by the consumer using CAS.
     */
    uint64_t consumer_seq;

    /**
     * Producer sequence — next slot to write.
     * Advanced by the producer (DoLogger).
     */
    uint64_t producer_seq;

    /** Total records dropped due to buffer full. */
    uint64_t dropped_count;

    /** Total records overwritten (drop_oldest strategy). */
    uint64_t overwritten_count;

    /** Magic number (DOLOGGER_SHM_MAGIC). */
    uint32_t magic;

    /** Layout version (DOLOGGER_SHM_VERSION). */
    uint32_t version;

    /** Number of slots in the ring buffer. */
    uint32_t slot_count;

    /** Size of each slot in bytes (includes 4B length prefix). */
    uint32_t slot_size_bytes;

    /** Producer process ID. */
    uint32_t producer_pid;

    /** Flags bitmask (see DOLOGGER_SHM_FLAG_*). */
    uint32_t flags;
} dologger_shm_header_t;

/* Compile-time size check */
#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(sizeof(dologger_shm_header_t) == 64,
               "dologger_shm_header_t must be exactly 64 bytes");
#endif

/* -------------------------------------------------------------------------
 * Slot layout
 * ------------------------------------------------------------------------- */

/**
 * Each slot in the ring buffer has this structure:
 *
 * Offset | Size | Field
 * -------|------|------------------------
 * 0      | 4    | record_len (uint32_t LE)
 * 4      | N    | SIF binary data (N bytes)
 * 4+N    | pad  | padding to slot_size_bytes
 *
 * where `record_len` is the total length of the SIF data in bytes,
 * stored as little-endian uint32_t.
 *
 * The SIF binary format starts with magic "SIF\0" (4 bytes), followed by
 * a fixed header and a KV-backed record payload. All integers are little-endian.
 */

/** Maximum SIF record size supported (matches the simplified SIF). */
#define DOLOGGER_SHM_MAX_RECORD_SIZE 65536

/** Offset of the length prefix within a slot. */
#define DOLOGGER_SHM_SLOT_LEN_OFFSET  0
/** Offset of the SIF data within a slot. */
#define DOLOGGER_SHM_SLOT_DATA_OFFSET 4

/* -------------------------------------------------------------------------
 * SIF record frame (KV-backed serialization)
 * ------------------------------------------------------------------------- */

/**
 * The SIF (Standard Intermediate Format) payload written into each slot is a
 * single-record SIF frame built from fixed metadata and KV entries. The
 * encoder/decoder live in `core/src/sif/` (Rust). This header documents the
 * on-wire frame boundary so consumers can locate and validate the payload.
 *
 * Frame layout (all multi-byte integers little-endian):
 *   [0..3]    magic: "SIF\0"
 *   [4..5]    header_len: fixed SIF header length
 *   [6..7]    flags: payload and integrity flags
 *   [8..11]   total_len: complete frame length
 *   [12..15]  field_count: number of dynamic KV entries
 *   [16..19]  fixed_len: fixed record metadata length
 *   [20..31]  reserved: zero-filled for deterministic framing
 *   [32..]    fixed record metadata, raw message bytes, and KV entries
 *
 * Consumers MUST validate all lengths, limits, field names, types, and the
 * content hash when the frame requests integrity verification. KV is the
 * internal dynamic-field organization; SIF is the transport boundary.
 */

/** Magic bytes for the SIF frame ("SIF\0" — first 4 bytes of the payload). */
#define DOLOGGER_SIF_MAGIC 0x00464953u

/** SIF fixed header size, including the four-byte magic. */
#define DOLOGGER_SIF_FRAME_OVERHEAD 32u

#ifdef __cplusplus
}
#endif

#endif /* DOLOGGER_SHM_H */
