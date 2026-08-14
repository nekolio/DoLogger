/* demo/snippets.ts — the four "before migration" source files shown on
 * page 2. Each file is a realistic, runnable-looking user service whose
 * only audit trail is stdout (println! / fmt.Printf / print / fprintf) —
 * exactly the shape you find in real open-source projects before they
 * adopt a structured logger. The demo animates replacing that one line
 * with a DoLogger AUDIT record.
 *
 * The API calls mirror the real adapters in this repo:
 *   adapters/rust/src/lib.rs     Logger::init(None) + info/warn/audit/shutdown
 *   adapters/go/dologger.go      dologger.NewLogger("") + Info/Warn/Audit + Shutdown
 *   adapters/python/dologger.py  DoLogger() + info/warn/audit/shutdown
 *   core/src/ffi.rs              dologger_init / dologger_log / dologger_shutdown
 *
 * NOTE: inside the template literals every `\n` in the source code is
 * written as `\\n` so it survives JS string parsing.
 */

import { lexers } from './tokenizer'
import type { LangLexer } from './tokenizer'

export interface Snippet {
  lang: string
  file: string
  /** One line per array entry — the scrolling code panel. */
  lines: string[]
  /** Index of the line the demo edits (found by marker, not by hand). */
  targetLine: number
  /** The migrated replacement line (typed character-by-character). */
  replaceLine: string
  /** "Before" pill label shown in the terminal header. */
  before: string
  /** "After" pill label shown once the migration completes. */
  after: string
  lexer: LangLexer
}

function makeSnippet(
  lang: string, file: string, src: string, marker: string,
  replaceLine: string, before: string, after: string
): Snippet {
  const lines = src.split('\n')
  const targetLine = lines.findIndex(l => l.includes(marker))
  if (targetLine < 0) throw new Error('demo: marker not found in ' + file)
  return { lang, file, lines, targetLine, replaceLine, before, after, lexer: lexers[lang] }
}

/* ─────────────────────────── Rust · main.rs ──────────────────────── */

const RUST_SRC = `// main.rs — user-service: a minimal async TCP service.
//
// The service keeps an in-memory user store and answers one command per
// line ("PING" | "DEL <id>"). This is the "before" shape seen in the
// wild: the only audit trail is a println! to stdout — it dies with the
// process, cannot be replayed, and nobody can prove who deleted record
// #7. The migration swaps that one line for a DoLogger AUDIT record:
// Ed25519-signed, hash-chained, and written to WORM storage.
//
// Run: cargo run --release

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use dologger_sdk::Logger;

/// In-memory user store, keyed by user id.
#[derive(Default)]
struct UserStore {
    users: HashMap<u64, String>, // id -> display name
}

impl UserStore {
    /// Remove a user; returns the display name that was stored.
    fn delete(&mut self, uid: u64) -> Option<String> {
        self.users.remove(&uid)
    }

    /// List all stored users as "id=name" pairs.
    fn snapshot(&self) -> Vec<(u64, String)> {
        let mut out: Vec<(u64, String)> = self.users.iter().map(|(k, v)| (*k, v.clone())).collect();
        out.sort_unstable();
        out
    }
}

/// Handle one client connection: read command lines until EOF.
async fn handle_conn(
    mut stream: TcpStream,
    store: Arc<Mutex<UserStore>>,
    logger: Arc<Mutex<Logger>>,
) {
    let (reader, mut writer) = stream.split();
    let mut lines = BufReader::new(reader).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let parts: Vec<&str> = line.trim().split(' ').collect();
        match parts.as_slice() {
            // Health probe — no logging needed.
            ["PING"] => {
                let _ = writer.write_all(b"PONG\\n").await;
            }

            // List users — trivial read, still worth an INFO line.
            ["LS"] => {
                let users = store.lock().await.snapshot();
                let mut buf = String::new();
                for (id, name) in users {
                    buf.push_str(&format!("{id}={name}\\n"));
                }
                let _ = writer.write_all(buf.as_bytes()).await;
            }

            // Delete a user — this is the operation that MUST be audited.
            ["DEL", id] => match id.parse::<u64>() {
                Ok(uid) => {
                    let deleted = store.lock().await.delete(uid);
                    match deleted {
                        Some(_name) => {
                            // BEFORE: stdout is neither durable nor auditable —
                            // lost on restart, unsigned, no integrity chain.
                            println!("user {} deleted record #{}", uid, 7);
                        }
                        None => {
                            logger.lock().await.warn(&format!("delete failed: unknown user {uid}"));
                        }
                    }
                }
                Err(_) => {
                    logger.lock().await.warn("malformed DEL payload");
                }
            },

            // Anything else — reject with a warning.
            _ => {
                logger.lock().await.warn("unknown command");
            }
        }
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // DoLogger engine, default config (auto-discovery): zero-copy ring
    // buffer, 7-stage pipeline, 11 sinks, Ed25519 audit chain — one line
    // to start it all.
    let logger = Arc::new(Mutex::new(
        dologger_sdk::Logger::init(None).expect("dologger init"),
    ));

    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
    let listener = TcpListener::bind(addr).await?;
    logger.lock().await.info(&format!("listening on {addr}"));

    // Seed a couple of users so DEL has something to delete.
    let store = Arc::new(Mutex::new(UserStore::default()));
    store.lock().await.users.insert(42, "ada".into());
    store.lock().await.users.insert(7, "grace".into());

    loop {
        let (stream, peer) = listener.accept().await?;
        let store = store.clone();
        let logger = logger.clone();
        logger.lock().await.info(&format!("accepted {peer}"));
        tokio::spawn(async move {
            handle_conn(stream, store, logger).await;
        });
    }
    // NOTE: a production service would also handle SIGINT and call
    // logger.shutdown() so the pipeline flushes its remaining records.
}`

/* ─────────────────────────── Go · main.go ────────────────────────── */

const GO_SRC = `// main.go — user-service: HTTP API with an in-memory user store.
//
// The "before" shape: stdlib log + fmt.Printf to stdout. The only audit
// trail of a destructive DELETE is a line of text that no one stores,
// signs, or replays. After the migration that line becomes a DoLogger
// Audit record — non-repudiable and WORM-persisted.
//
// Run: go run .

package main

import (
    "context"
    "encoding/json"
    "errors"
    "fmt"
    "log"
    "net/http"
    "os"
    "os/signal"
    "strconv"
    "strings"
    "sync"
    "syscall"
    "time"

    "github.com/dologger/adapters/go/dologger"
)

// userStore is a tiny thread-safe user table.
type userStore struct {
    mu    sync.RWMutex
    users map[uint64]string // id -> display name
}

func newUserStore() *userStore {
    return &userStore{users: make(map[uint64]string)}
}

// deleteUser removes the user and reports whether it existed.
func (s *userStore) deleteUser(id uint64) (string, bool) {
    s.mu.Lock()
    defer s.mu.Unlock()
    name, ok := s.users[id]
    if ok {
        delete(s.users, id)
    }
    return name, ok
}

// requestLogger wraps a handler with elapsed-time logging.
func requestLogger(logger *dologger.Logger, next http.Handler) http.Handler {
    return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
        start := time.Now()
        next.ServeHTTP(w, r)
        logger.Info(fmt.Sprintf("%s %s %s", r.Method, r.URL.Path, time.Since(start)))
    })
}

func main() {
    // DoLogger engine with auto-discovered config.
    logHandle, err := dologger.NewLogger("")
    if err != nil {
        panic(err)
    }
    defer logHandle.Shutdown()

    store := newUserStore()
    store.mu.Lock()
    store.users[42] = "ada"
    store.users[7] = "grace"
    store.mu.Unlock()

    mux := http.NewServeMux()

    // Health probe.
    mux.HandleFunc("GET /ping", func(w http.ResponseWriter, r *http.Request) {
        fmt.Fprintln(w, "PONG")
    })

    // List users.
    mux.HandleFunc("GET /users", func(w http.ResponseWriter, r *http.Request) {
        store.mu.RLock()
        defer store.mu.RUnlock()
        _ = json.NewEncoder(w).Encode(store.users)
    })

    // Delete a user — the operation that MUST be audited.
    mux.HandleFunc("DELETE /users/{id}", func(w http.ResponseWriter, r *http.Request) {
        id, err := strconv.ParseUint(r.PathValue("id"), 10, 64)
        if err != nil {
            http.Error(w, "bad id", http.StatusBadRequest)
            return
        }
        _, ok := store.deleteUser(id)
        if !ok {
            http.NotFound(w, r)
            return
        }
        // BEFORE: this line is the entire audit trail — unlogged by
        // any sink, unsigned, unreplayable after a restart.
        fmt.Printf("user %d deleted record #%d\\n", id, 7)
        w.WriteHeader(http.StatusNoContent)
    })

    server := &http.Server{Addr: ":8080", Handler: requestLogger(logHandle, mux)}
    log.Printf("user-service listening on :8080")

    // Graceful shutdown on SIGINT/SIGTERM.
    ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
    defer stop()
    go func() {
        if err := server.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
            log.Fatalf("server: %v", err)
        }
    }()
    <-ctx.Done()
    shutdownCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
    defer cancel()
    _ = server.Shutdown(shutdownCtx)
    logHandle.Info("http server stopped, pipeline flushed")
}`

/* ───────────────────────── Python · main.py ──────────────────────── */

const PY_SRC = `"""user-service — minimal asyncio TCP server with an in-memory user store.

The "before" shape: the only audit trail is print() to stdout. After the
migration that line becomes a DoLogger audit() call — signed with
Ed25519, chained by LSN + prev_hash, and persisted to WORM storage.

Run: python -m user_service
"""

import asyncio
from collections import OrderedDict


class UserStore:
    """In-memory user store with a deliberately small API surface."""

    def __init__(self) -> None:
        self._users: "OrderedDict[int, str]" = OrderedDict()

    def seed(self, uid: int, name: str) -> None:
        """Insert a user (used for demo fixtures)."""
        self._users[uid] = name

    def delete(self, uid: int) -> str | None:
        """Remove a user; return the stored name or None."""
        return self._users.pop(uid, None)

    def snapshot(self) -> list[tuple[int, str]]:
        """Return all users as (id, name) pairs."""
        return list(self._users.items())


async def handle_conn(
    reader: asyncio.StreamReader,
    writer: asyncio.StreamWriter,
    store: UserStore,
    log,
) -> None:
    """Read one command per line: PING | LS | DEL <id>."""
    while True:
        raw = await reader.readline()
        if not raw:
            break  # client closed the connection
        line = raw.decode().strip()
        parts = line.split(" ")

        if parts[0] == "PING":
            writer.write(b"PONG\\n")
            await writer.drain()

        elif parts[0] == "LS":
            snapshot = store.snapshot()
            body = "".join(f"{uid}={name}\\n" for uid, name in snapshot)
            writer.write(body.encode())
            await writer.drain()

        elif parts[0] == "DEL" and len(parts) == 2:
            try:
                uid = int(parts[1])
            except ValueError:
                print(f"[warn] malformed DEL payload: {line!r}")
                continue
            name = store.delete(uid)
            if name is None:
                print(f"[warn] delete failed: unknown user {uid}")
                continue
            # BEFORE: this print() is the entire audit trail — no
            # signature, no chain, gone with the process.
            print(f"user {uid} deleted record #7")

        else:
            print(f"[warn] unknown command: {line!r}")

    writer.close()


async def main() -> None:
    """Start the TCP listener on 127.0.0.1:8080."""
    store = UserStore()
    store.seed(42, "ada")
    store.seed(7, "grace")

    # DoLogger with auto-discovered config.
    from dologger import DoLogger
    log = DoLogger()

    server = await asyncio.start_server(
        lambda r, w: handle_conn(r, w, store, log),
        host="127.0.0.1",
        port=8080,
    )
    print(f"[info] user-service listening on 127.0.0.1:8080")

    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("[info] shutting down")`

/* ─────────────────────────── C · main.c ──────────────────────────── */

const C_SRC = `/*
 * main.c — user-service: a tiny TCP event loop in the style of classic
 * C daemons, with an in-memory user store.
 *
 * The "before" shape: fprintf to stdout is the entire audit trail of a
 * destructive DELETE. The migration replaces that call with an AUDIT
 * record through the DoLogger C ABI (core/src/ffi.rs).
 *
 * Build: gcc -O2 main.c -ldologger_core -o user-service
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <arpa/inet.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <unistd.h>

#include "dologger_core.h"

#define PORT        8080
#define MAX_LINE    256
#define MAX_USERS   64

/* In-memory user store (id -> name). */
static uint64_t g_user_ids[MAX_USERS];
static char     g_user_names[MAX_USERS][MAX_LINE];
static size_t   g_user_count = 0;

/* Insert or update a user by id. */
static void user_upsert(uint64_t id, const char *name) {
    for (size_t i = 0; i < g_user_count; i++) {
        if (g_user_ids[i] == id) {
            snprintf(g_user_names[i], sizeof g_user_names[i], "%s", name);
            return;
        }
    }
    if (g_user_count < MAX_USERS) {
        g_user_ids[g_user_count] = id;
        snprintf(g_user_names[g_user_count], sizeof g_user_names[g_user_count], "%s", name);
        g_user_count++;
    }
}

/* Delete a user; returns 1 if the id existed. */
static int user_delete(uint64_t id) {
    for (size_t i = 0; i < g_user_count; i++) {
        if (g_user_ids[i] == id) {
            memmove(&g_user_ids[i], &g_user_ids[i + 1],
                    (g_user_count - i - 1) * sizeof(uint64_t));
            memmove(&g_user_names[i], &g_user_names[i + 1],
                    (g_user_count - i - 1) * sizeof(g_user_names[0]));
            g_user_count--;
            return 1;
        }
    }
    return 0;
}

/* Parse and execute one command line: PING | DEL <id>. */
static void handle_command(const char *line, int out_fd, dologger_handle_t *log) {
    char cmd[MAX_LINE];
    uint64_t id = 0;
    int n = sscanf(line, "%255s %llu", cmd, &id);

    if (n >= 1 && strcmp(cmd, "PING") == 0) {
        write(out_fd, "PONG\\n", 5);
        return;
    }

    if (n >= 2 && strcmp(cmd, "DEL") == 0) {
        if (!user_delete(id)) {
            dologger_log_params_t p = {0};
            p.level = DO_LOG_WARN;
            p.message = "delete failed: unknown user";
            dologger_log(log, &p);
            return;
        }
        /* BEFORE: this fprintf is the entire audit trail — unsigned,
         * unchained, and gone with the process. */
        fprintf(stdout, "user %llu deleted record #7\\n", id);
        return;
    }

    dologger_log_params_t p = {0};
    p.level = DO_LOG_WARN;
    p.message = "unknown command";
    dologger_log(log, &p);
}

int main(void) {
    /* DoLogger engine via the C ABI (default config). */
    dologger_error_t err;
    dologger_handle_t *log = dologger_init(NULL, &err);
    if (log == NULL) {
        fprintf(stderr, "dologger_init failed: %s\\n", err.message);
        return EXIT_FAILURE;
    }

    /* Demo fixtures. */
    user_upsert(42, "ada");
    user_upsert(7, "grace");

    /* Bind the listener. */
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) {
        perror("socket");
        return EXIT_FAILURE;
    }
    int yes = 1;
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &yes, sizeof yes);

    struct sockaddr_in addr = {0};
    addr.sin_family = AF_INET;
    addr.sin_port = htons(PORT);
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (bind(fd, (struct sockaddr *)&addr, sizeof addr) < 0) {
        perror("bind");
        return EXIT_FAILURE;
    }
    if (listen(fd, 16) < 0) {
        perror("listen");
        return EXIT_FAILURE;
    }
    fprintf(stderr, "user-service listening on 127.0.0.1:%d\\n", PORT);

    /* Accept loop — one connection at a time for the demo. */
    for (;;) {
        int client = accept(fd, NULL, NULL);
        if (client < 0) {
            perror("accept");
            continue;
        }
        char line[MAX_LINE];
        ssize_t got;
        while ((got = read(client, line, sizeof line - 1)) > 0) {
            line[got] = '\\0';
            handle_command(line, client, log);
        }
        close(client);
    }
    /* NOTE: a daemon would handle SIGTERM and call dologger_shutdown(log)
     * so the pipeline can flush its remaining records. */
}`

/* ─────────────────────────── exports ─────────────────────────────── */

export const demoFiles: { lang: string; file: string }[] = [
  { lang: 'rust', file: 'main.rs' },
  { lang: 'go', file: 'main.go' },
  { lang: 'python', file: 'main.py' },
  { lang: 'c', file: 'main.c' }
]

export const snippets: Record<string, Snippet> = {
  rust: makeSnippet(
    'rust', 'main.rs', RUST_SRC,
    'println!("user {} deleted',
    '                            logger.lock().await.audit(&format!("user {uid} deleted record #7")); // Ed25519-signed + WORM',
    'stdout only', 'DoLogger AUDIT'
  ),
  go: makeSnippet(
    'go', 'main.go', GO_SRC,
    'fmt.Printf("user %d deleted',
    '        logHandle.Audit(fmt.Sprintf("user %d deleted record #%d", id, 7))',
    'fmt.Printf', 'logHandle.Audit'
  ),
  python: makeSnippet(
    'python', 'main.py', PY_SRC,
    'print(f"user {uid} deleted',
    '            log.audit(f"user {uid} deleted record #7")',
    'print()', 'log.audit()'
  ),
  c: makeSnippet(
    'c', 'main.c', C_SRC,
    'fprintf(stdout, "user %llu deleted',
    '        dologger_log(log, &(dologger_log_params_t){ .level = DO_LOG_AUDIT, .message = "user 42 deleted record #7 — signed + WORM" });',
    'fprintf', 'dologger_log(AUDIT)'
  )
}
