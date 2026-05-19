/* kittui FFI — checked-in ABI snapshot.
 *
 * This header is the canonical declaration of the kittui C ABI. The
 * ABI snapshot test in `crates/kittui-ffi/tests/abi_snapshot.rs`
 * asserts that the symbols listed here are exported by the cdylib.
 *
 * Versioning: bump KITTUI_ABI_MINOR on additive changes, KITTUI_ABI_MAJOR
 * on breaking changes, and update both this file and the snapshot test
 * in the same commit. See DESIGN.md `## FFI → ABI versioning`.
 */

#ifndef KITTUI_FFI_H
#define KITTUI_FFI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Major ABI version. Bumped on breaking changes. */
#define KITTUI_ABI_MAJOR 0

/* Minor ABI version. Bumped on additive changes. */
#define KITTUI_ABI_MINOR 1

/* Return value for every fallible entry point. */
typedef enum KittuiStatus {
    KITTUI_STATUS_OK = 0,
    KITTUI_STATUS_NULL_POINTER = 1,
    KITTUI_STATUS_BAD_SCENE = 2,
    KITTUI_STATUS_RUNTIME = 3,
    KITTUI_STATUS_PANIC = 4,
} KittuiStatus;

/* Opaque runtime handle. */
typedef struct KittuiRuntime KittuiRuntime;

/* Return (major << 16) | minor. */
uint32_t kittui_abi_version(void);

/* Construct a runtime. cache_dir may be NULL to use the platform default. */
KittuiRuntime* kittui_runtime_new(const char* cache_dir);

/* Free a runtime allocated by kittui_runtime_new. */
void kittui_runtime_free(KittuiRuntime* runtime);

/* Render+place a scene supplied as JSON. On success writes a
 * heap-allocated NUL-terminated UTF-8 string into *out which the
 * caller must free with kittui_string_free. */
KittuiStatus kittui_place_json(KittuiRuntime* runtime,
                               const char* scene_json,
                               char** out);

/* Free a string returned by the FFI. */
void kittui_string_free(char* ptr);

#ifdef __cplusplus
}
#endif

#endif /* KITTUI_FFI_H */
