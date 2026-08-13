/* demo/tokenizer.ts — per-language one-pass syntax highlighter.
 *
 * The old demo highlighted with a chain of global regex `.replace()` calls,
 * which could wrap tokens twice and missed numbers / operators / macros /
 * attributes / decorators / preprocessor directives. This tokenizer walks
 * the line left-to-right, matches the longest rule at each position, and
 * emits exactly one span per token — no nesting, no double wrapping.
 *
 * Token classes map to CSS variables in style.css:
 *   kw, type, builtin, str, num, com, fn, op, macro, attr, deco, preproc,
 *   dologger (the migrated API), stdout (the legacy print), plain.
 */

export type TokClass =
  | 'kw' | 'type' | 'builtin' | 'str' | 'num' | 'com' | 'fn' | 'op'
  | 'macro' | 'attr' | 'deco' | 'preproc' | 'dologger' | 'stdout' | 'plain'

export interface LangLexer {
  /** Line-comment or block-comment regex, anchored at the current position. */
  comment: RegExp
  /** Quote char for strings ("). */
  strQuote: string
  /** Quote char for character literals ('), e.g. Rust / C. */
  charQuote?: string
  /** Number literal (hex / decimal / float / exponent / underscores). */
  number: RegExp
  keywords: Set<string>
  types: Set<string>
  builtins: Set<string>
  /** Rust: `ident!` macros (println!, format!). */
  macroSuffix: boolean
  /** Rust: `#[...]` / `#![...]` attributes. */
  attrPrefix: boolean
  /** C: `#include` / `#define`. */
  preprocPrefix: boolean
  /** Python: `@decorator`. */
  decoPrefix: boolean
  /** Identifiers belonging to the DoLogger brand. */
  dologgerIdents: Set<string>
  /** Identifiers that print to stdout (the "before" style). */
  stdoutIdents: Set<string>
  /** Color `ident(` as a function call. */
  fnCall: boolean
}

const OP_CHARS = '=+-*/%<>!?&|^~'

function esc(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

function span(cls: TokClass, text: string): string {
  return cls === 'plain' ? text : `<span class="tok-${cls}">${text}</span>`
}

/** Read a quoted literal starting at position i (backslash escapes honored). */
function readQuote(line: string, i: number, quote: string): string | null {
  let j = i + 1
  while (j < line.length) {
    if (line[j] === '\\') { j += 2; continue }
    if (line[j] === quote) return line.slice(i, j + 1)
    j++
  }
  return line.slice(i) // unterminated — highlight what we have
}

/** Tokenize one line of code for the given language. Returns HTML. */
export function tokenize(line: string, lex: LangLexer): string {
  if (!line) return ''
  let html = ''
  let i = 0
  const n = line.length
  while (i < n) {
    const rest = line.slice(i)

    /* comment */
    const cm = lex.comment.exec(rest)
    if (cm && cm.index === 0) {
      html += span('com', esc(cm[0]))
      i += cm[0].length
      continue
    }

    /* string literal */
    if (rest[0] === lex.strQuote) {
      const s = readQuote(line, i, lex.strQuote)
      if (s) { html += span('str', esc(s)); i += s.length; continue }
    }

    /* char literal (Rust / C) */
    if (lex.charQuote && rest[0] === lex.charQuote) {
      const s = readQuote(line, i, lex.charQuote)
      if (s) { html += span('str', esc(s)); i += s.length; continue }
    }

    /* number */
    const nm = lex.number.exec(rest)
    if (nm && nm.index === 0) {
      html += span('num', esc(nm[0]))
      i += nm[0].length
      continue
    }

    /* Rust attribute / C preprocessor / Python decorator */
    if (rest[0] === '#') {
      if (lex.attrPrefix) {
        const end = rest.search(/\]/); const t = end >= 0 ? rest.slice(0, end + 1) : rest
        html += span('attr', esc(t)); i += t.length; continue
      }
      if (lex.preprocPrefix) {
        const end = rest.search(/\n/); const t = end >= 0 ? rest.slice(0, end) : rest
        html += span('preproc', esc(t)); i += t.length; continue
      }
      html += span('op', '#'); i++; continue
    }
    if (rest[0] === '@' && lex.decoPrefix) {
      const wm = /^@[A-Za-z_][A-Za-z0-9_]*(\([^)]*\))?/.exec(rest)
      if (wm) { html += span('deco', esc(wm[0])); i += wm[0].length; continue }
    }

    /* identifier / keyword */
    const wm = /^[A-Za-z_][A-Za-z0-9_]*/.exec(rest)
    if (wm) {
      const w = wm[0]
      const after = rest[wm[0].length] || ''
      let cls: TokClass = 'plain'
      if (lex.keywords.has(w)) cls = 'kw'
      else if (lex.types.has(w)) cls = 'type'
      else if (lex.builtins.has(w)) cls = 'builtin'
      else if (lex.dologgerIdents.has(w)) cls = 'dologger'
      else if (lex.stdoutIdents.has(w)) cls = 'stdout'
      else if (lex.macroSuffix && after === '!') cls = 'macro'
      else if (lex.fnCall && after === '(') cls = 'fn'
      if (cls === 'stdout' && after === '!') {
        html += span('stdout', esc(w + '!'))
        i += w.length + 1
        continue
      }
      html += span(cls, esc(w))
      i += w.length
      continue
    }

    /* operator */
    if (OP_CHARS.includes(rest[0])) {
      html += span('op', esc(rest[0]))
      i++
      continue
    }

    /* everything else verbatim */
    html += esc(rest[0])
    i++
  }
  return html
}

/* ------------------------------------------------------------------ */
/* Language lexers — vocabularies mirror real snippets, not the full   */
/* grammar (a demo, not rust-analyzer).                                */
/* ------------------------------------------------------------------ */

const set = (words: string): Set<string> => new Set(words.split(' '))

export const lexers: Record<string, LangLexer> = {
  rust: {
    comment: /^\/\/[^\n]*/,
    strQuote: '"',
    charQuote: "'",
    number: /^0x[0-9a-fA-F_]+|\d[\d_]*(\.\d+)?([eE][+-]?\d+)?/,
    keywords: set('as async await break const continue crate dyn else enum extern fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait type unsafe use where while'),
    types: set('u8 u16 u32 u64 i8 i16 i32 i64 usize isize f32 f64 bool char str String Vec Option Result HashMap'),
    builtins: set('Ok Err None Some'),
    macroSuffix: true,
    attrPrefix: true,
    preprocPrefix: false,
    decoPrefix: false,
    dologgerIdents: set('Logger dologger_sdk dologger_core Logger::init init'),
    stdoutIdents: set('println print eprintln'),
    fnCall: true
  },
  go: {
    comment: /^\/\/[^\n]*/,
    strQuote: '"',
    charQuote: "'",
    number: /^0x[0-9a-fA-F_]+|\d[\d_]*(\.\d+)?([eE][+-]?\d+)?/,
    keywords: set('break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var'),
    types: set('string int int64 uint64 uint32 float64 bool byte rune error nil any error http.ResponseWriter *http.Request'),
    builtins: set('make new len cap append panic recover'),
    macroSuffix: false,
    attrPrefix: false,
    preprocPrefix: false,
    decoPrefix: false,
    dologgerIdents: set('dologger NewLogger Info Warn Error Debug Trace Fatal Audit Shutdown Logger'),
    stdoutIdents: set('fmt.Println fmt.Printf fmt.Print println print'),
    fnCall: true
  },
  python: {
    comment: /^#[^\n]*/,
    strQuote: '"',
    charQuote: undefined,
    number: /^\d[\d_]*(\.\d+)?([eE][+-]?\d+)?/,
    keywords: set('and as assert async await break class continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return try while with yield'),
    types: set('int float bool str bytes list dict tuple set None True False'),
    builtins: set('asyncio StreamReader StreamWriter OrderedDict range len int str print'),
    macroSuffix: false,
    attrPrefix: false,
    preprocPrefix: false,
    decoPrefix: true,
    dologgerIdents: set('DoLogger dologger'),
    stdoutIdents: set('print'),
    fnCall: true
  },
  c: {
    comment: /^\/\/[^\n]*|\/\*[\s\S]*?\*\//,
    strQuote: '"',
    charQuote: "'",
    number: /^0x[0-9a-fA-F]+|\d[\d_]*\.?\d*([eE][+-]?\d+)?[uUlLfF]*/,
    keywords: set('auto break case char const continue default do double else enum extern float for goto if inline int long register restrict return short signed sizeof static struct switch typedef union unsigned void volatile while'),
    types: set('uint8_t uint16_t uint32_t uint64_t int32_t int64_t size_t ssize_t socklen_t in_addr_t sockaddr sockaddr_in socklen_t'),
    builtins: set('NULL EXIT_FAILURE EXIT_SUCCESS true false'),
    macroSuffix: false,
    attrPrefix: false,
    preprocPrefix: true,
    decoPrefix: false,
    dologgerIdents: set('dologger_init dologger_log dologger_shutdown dologger_get_last_error dologger_version dologger_error_t dologger_handle_t dologger_log_params_t DO_LOG_TRACE DO_LOG_DEBUG DO_LOG_INFO DO_LOG_WARN DO_LOG_ERROR DO_LOG_FATAL DO_LOG_AUDIT'),
    stdoutIdents: set('printf fprintf fputs puts'),
    fnCall: true
  }
}
