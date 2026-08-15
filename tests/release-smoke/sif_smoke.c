/* SIF C ABI smoke test — verifies dologger_core.h compiles as C and the
 * SIF encode/decode/validate surface behaves end-to-end.
 *
 * Usage (Windows MSVC):
 *   cl /nologo /W3 /Isrc/..\.. /Isrc\core\include sif_smoke.c /link dologger_core.lib
 *   sif_smoke.exe
 *
 * The test returns 0 on success, non-zero on failure.
 */
#include <stdio.h>
#include <string.h>
#include "dologger_core.h"

int main(void) {
    dologger_error_t err;
    memset(&err, 0, sizeof(err));

    /* 1. Create a record and populate a Ring-3 field. */
    dologger_record_t *rec = dologger_record_create();
    if (rec == NULL) {
        printf("FAIL: dologger_record_create returned NULL\n");
        return 1;
    }
    if (dologger_field_set(rec, "ext.trace_id", "abc123", &err) != 0) {
        printf("FAIL: dologger_field_set: code=%d\n", err.code);
        return 1;
    }

    /* 2. Encode to a SIF frame. */
    uint8_t *frame = NULL;
    size_t frame_len = 0;
    if (dologger_sif_encode_record(rec, &frame, &frame_len, &err) != 0) {
        printf("FAIL: dologger_sif_encode_record: code=%d\n", err.code);
        return 1;
    }
    if (frame == NULL || frame_len == 0) {
        printf("FAIL: encode returned empty frame\n");
        return 1;
    }

    /* 3. Validate the frame. */
    if (dologger_sif_validate_frame(frame, frame_len, &err) != 0) {
        printf("FAIL: dologger_sif_validate_frame: code=%d\n", err.code);
        return 1;
    }

    /* 4. Corrupt the magic -> validation must fail with DO_LOG_ERR_SIF_INVALID. */
    {
        uint8_t saved = frame[0];
        frame[0] = 'X';
        int rc = dologger_sif_validate_frame(frame, frame_len, &err);
        if (rc != DO_LOG_ERR_SIF_INVALID) {
            printf("FAIL: corrupted frame returned rc=%d (expected DO_LOG_ERR_SIF_INVALID)\n", rc);
            return 1;
        }
        frame[0] = saved;
    }

    /* 5. Decode back into a record and read the field. */
    dologger_record_t *decoded = NULL;
    if (dologger_sif_decode_record(frame, frame_len, &decoded, &err) != 0) {
        printf("FAIL: dologger_sif_decode_record: code=%d\n", err.code);
        return 1;
    }
    if (decoded == NULL) {
        printf("FAIL: decode returned NULL record\n");
        return 1;
    }
    {
        char buf[64];
        int n = dologger_field_get(decoded, "ext.trace_id", buf, sizeof(buf), &err);
        if (n != 6 || strncmp(buf, "abc123", 6) != 0) {
            printf("FAIL: decoded field mismatch: n=%d buf='%.*s'\n", n, n < 0 ? 0 : n, buf);
            return 1;
        }
    }

    /* 6. Cleanup. */
    dologger_record_destroy(rec);
    dologger_record_destroy(decoded);
    dologger_free(frame);

    printf("PASS: SIF C ABI smoke test\n");
    return 0;
}
