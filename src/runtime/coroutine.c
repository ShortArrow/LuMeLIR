/* ADR 0315 — N5-A: coroutine runtime core.
 *
 * C (not the no_std Rust bridge) because makecontext/swapcontext
 * need the libc-private ucontext_t layout; <ucontext.h> is the only
 * stable way to fill uc_stack/uc_link. Compiled by build.rs with the
 * host cc and linked into every generated binary next to
 * bridge_runtime.o.
 *
 * N5-A scope: allocation + status only. Lua's coroutine.create runs
 * nothing — the makecontext trampoline belongs to resume (N5-B).
 */
#include <stdint.h>
#include <stdlib.h>
#include <ucontext.h>

#define LUMELIR_CORO_STACK_SIZE (256 * 1024)

enum {
    LUMELIR_CORO_SUSPENDED = 0,
    LUMELIR_CORO_RUNNING = 1,
    LUMELIR_CORO_DEAD = 2,
    LUMELIR_CORO_NORMAL = 3,
};

typedef struct lumelir_coro {
    ucontext_t self;
    ucontext_t caller;
    void *stack;
    int64_t status;
    void (*fn_ptr)(void *);
    void *cell_ptr;
} lumelir_coro;

void *lumelir_coro_create(void *fn_ptr, void *cell_ptr) {
    lumelir_coro *co = (lumelir_coro *)malloc(sizeof(lumelir_coro));
    if (co == NULL) {
        return NULL;
    }
    co->stack = malloc(LUMELIR_CORO_STACK_SIZE);
    if (co->stack == NULL) {
        free(co);
        return NULL;
    }
    co->status = LUMELIR_CORO_SUSPENDED;
    co->fn_ptr = (void (*)(void *))fn_ptr;
    co->cell_ptr = cell_ptr;
    return co;
}

int64_t lumelir_coro_status(void *co_ptr) {
    return ((lumelir_coro *)co_ptr)->status;
}
