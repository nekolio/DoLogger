/**
 * dologger_shm.h — Shared Memory Sink consumer API.
 *
 * This header defines the shared memory layout used by sink_shm.
 * External consumer processes (monitoring agents, TUI dashboards, etc.)
 * use these structures to attach to the shared memory ring buffer
 * and read SIF-formatted records with zero copy.
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
 *        // process sif_data[0..record_len] as SIF
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
 * The SIF binary format starts with magic "SIF1" (4 bytes) followed
 * by a little-endian uint32_t total_length, then the record fields.
 * See the SIF (Standard Intermediate Format) specification for the full binary layout.
 */

/** Maximum SIF record size supported (matches the simplified SIF). */
#define DOLOGGER_SHM_MAX_RECORD_SIZE 65536

/** Offset of the length prefix within a slot. */
#define DOLOGGER_SHM_SLOT_LEN_OFFSET  0
/** Offset of the SIF data within a slot. */
#define DOLOGGER_SHM_SLOT_DATA_OFFSET 4

/* -------------------------------------------------------------------------
 * SIF record frame (standard — FlatBuffers)
 * ------------------------------------------------------------------------- */

/**
 * The SIF (Standard Intermediate Format) payload written into each slot is a
 * single-record SIF frame, NOT a hand-rolled fixed-layout struct. The full
 * schema is defined in `core/sif/dologger_sif.fbs` and the encoder/decoder
 * live in `core/src/sif/` (Rust). This header only documents the on-wire
 * frame boundary so consumers can locate and validate the payload.
 *
 * Frame layout (all multi-byte integers little-endian):
 *   [0..3]    Magic "SIF1"
 *   [4..7]    version    — schema version (MAJOR<<24|MINOR<<16|PATCH); 1.0.0
 *   [8..11]   total_length — total SIF frame length (magic + header + payload)
 *   [12..15]  record_count — number of Record tables (1 for single record)
 *   [16..]    FlatBuffer payload (root type `Record`)
 *
 * The 16-byte frame overhead is `SIF_MAGIC` (4) + `SifHeader` (12).
 * `total_length` includes the 4 magic bytes + 12 header bytes + payload.
 *
 * Consumers MUST validate the magic bytes (`"SIF1"`) and the schema version
 * before interpreting the FlatBuffer payload. For version-independent field
 * access, parse the payload with the matching FlatBuffers-generated code for
 * the `Record` schema in `core/sif/dologger_sif.fbs`.
 */

/** Magic bytes for the SIF frame ("SIF1" — first 4 bytes of the payload). */
#define DOLOGGER_SIF_MAGIC 0x31464953u

/** SIF frame overhead: 4-byte magic + 12-byte SifHeader. */
#define DOLOGGER_SIF_FRAME_OVERHEAD 16u

/** Current SIF schema version, packed (1.0.0). */
#define DOLOGGER_SIF_VERSION 0x01000000u

#ifdef __cplusplus
}
#endif

#endif /* DOLOGGER_SHM_H */
