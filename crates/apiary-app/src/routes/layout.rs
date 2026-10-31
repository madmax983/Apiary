//! The shell every page renders into, and Apiary's look.
//!
//! A hive at night: wax-dark ground, and the only light is honey. A cell is
//! lit while its file is open, an ember for a moment after it closes, dark
//! wax at rest, and a ghost once deleted. Nothing else on the page is that
//! colour, so the eye goes straight to which files are open.
//!
//! Inline styles only; the one script is served from its own route because
//! Autumn's CSP is `script-src 'self'`.

use autumn_web::{HTMX_CSRF_JS_PATH, HTMX_JS_PATH, HTMX_SSE_JS_PATH, Markup, PreEscaped, html};

/// The stylesheet.
pub const STYLE: &str = r#"
:root{
  --night:#0d0a06; --night-2:#14100a; --panel:#18120b; --rule:#2c2214; --rule-2:#3d2f1b;
  --wax:#2a2013; --wax-2:#33271a; --ink:#efe3cc; --dim:#a8977a; --faint:#76684f;
  --honey:#f5a623; --honey-hi:#ffd479; --honey-lo:#c4700c; --ember:#7e3f10;
  --ghost:#1d1710; --bad:#e2553d; --good:#8fbf6a;
  --serif: "Iowan Old Style","Palatino Linotype",Palatino,Georgia,ui-serif,serif;
  --sans: ui-sans-serif,system-ui,-apple-system,"Segoe UI",Roboto,sans-serif;
  --mono: ui-monospace,SFMono-Regular,Menlo,"Cascadia Mono",monospace;
}
*{box-sizing:border-box}
html,body{margin:0;padding:0}
body{background:
  radial-gradient(1200px 700px at 30% -10%, rgba(245,166,35,.07), transparent 60%),
  radial-gradient(900px 600px at 100% 100%, rgba(196,112,12,.05), transparent 60%),
  var(--night);
  background-attachment:fixed;color:var(--ink);font-family:var(--sans);font-size:15px;line-height:1.55;min-height:100vh}
a{color:var(--honey);text-decoration:none} a:hover{text-decoration:underline}
code,.mono{font-family:var(--mono);font-size:.92em}
.wrap{max-width:1360px;margin:0 auto;padding:0 22px 60px}
header.top{display:flex;align-items:baseline;gap:18px;flex-wrap:wrap;padding:22px 0 14px;border-bottom:1px solid var(--rule);margin-bottom:22px}
header.top h1{font-family:var(--serif);font-weight:400;letter-spacing:.42em;text-transform:uppercase;font-size:25px;margin:0;color:var(--honey-hi)}
header.top h1 a{color:inherit} header.top h1 a:hover{text-decoration:none}
header.top .tag{color:var(--dim);font-family:var(--serif);font-style:italic;font-size:14px}
header.top nav{margin-left:auto;display:flex;gap:20px;font-size:13.5px}
header.top nav a{color:var(--dim)} header.top nav a:hover{color:var(--honey)}
h2{font-family:var(--serif);font-weight:400;font-size:27px;margin:0 0 6px;color:var(--ink)}
h3{font-size:11px;letter-spacing:.2em;text-transform:uppercase;color:var(--faint);margin:0 0 10px;font-weight:600}
p.lede{color:var(--dim);max-width:72ch;margin:0 0 18px}
.hint{color:var(--faint);font-size:12.5px}
.grid{display:grid;grid-template-columns:minmax(0,1fr) 340px;gap:22px;align-items:start}
@media (max-width:1100px){.grid{grid-template-columns:1fr}}
.panel{background:linear-gradient(180deg,rgba(255,255,255,.015),transparent),var(--panel);border:1px solid var(--rule);border-radius:6px;padding:16px 18px 18px}
.panel + .panel{margin-top:16px}
.stats{display:grid;grid-template-columns:repeat(2,1fr);gap:10px}
.stat{background:var(--night-2);border:1px solid var(--rule);border-radius:5px;padding:9px 11px}
.stat b{display:block;font-family:var(--serif);font-weight:400;font-size:23px;color:var(--ink);line-height:1.15}
.stat span{font-size:11px;color:var(--faint);letter-spacing:.06em}
.stat.lit b{color:var(--honey-hi)}
.gauge{height:10px;border-radius:5px;background:var(--wax);overflow:hidden;margin:6px 0 2px;border:1px solid var(--rule-2)}
.gauge i{display:block;height:100%;background:linear-gradient(90deg,var(--honey-lo),var(--honey),var(--honey-hi));box-shadow:0 0 12px rgba(245,166,35,.6)}
.btn{display:inline-block;font:inherit;font-size:14px;border:1px solid var(--honey-lo);background:linear-gradient(180deg,#3a2810,#271a09);color:var(--honey-hi);padding:8px 15px;border-radius:5px;cursor:pointer}
.btn:hover{border-color:var(--honey);box-shadow:0 0 14px rgba(245,166,35,.25)}
.btn[disabled]{opacity:.45;cursor:not-allowed;box-shadow:none}
.btn.quiet{border-color:var(--rule-2);color:var(--dim);background:var(--night-2)}
.btn.danger{border-color:#7a2a1c;color:#ffb7a6;background:linear-gradient(180deg,#3a1710,#260d08)}
input[type=text],textarea{font:inherit;background:var(--night-2);color:var(--ink);border:1px solid var(--rule-2);border-radius:5px;padding:8px 10px;width:100%}
input[type=text]:focus,textarea:focus{outline:none;border-color:var(--honey-lo);box-shadow:0 0 0 3px rgba(245,166,35,.12)}
form.row{display:flex;gap:8px} form.row input{flex:1}
.err{color:var(--bad);font-size:13px;margin-top:6px}
.ok{color:var(--good);font-size:13px;margin-top:6px}
.comb{padding:14px 10px 18px;overflow-x:auto}
.comb .r{display:flex;height:19px}
.comb .r.o{margin-left:11px}
.t{display:block;width:22px;height:25px;margin-right:0;flex:none;position:relative}
.t i{position:absolute;inset:1px;clip-path:polygon(50% 0,100% 25%,100% 75%,50% 100%,0 75%,0 25%);background:var(--wax)}
.t.cold i{background:linear-gradient(180deg,var(--wax-2),var(--wax))}
.t.warm{filter:drop-shadow(0 0 5px rgba(255,190,80,.85));z-index:2}
.t.warm i{background:radial-gradient(circle at 40% 35%,var(--honey-hi),var(--honey) 55%,var(--honey-lo))}
.t.cooling i{background:radial-gradient(circle at 40% 35%,#b5601a,var(--ember))}
.t.gone i{background:var(--ghost);opacity:.55}
.t:hover i{outline:none;background:var(--honey-hi)}
.t.me i{box-shadow:inset 0 0 0 2px #fff}
.legend{display:flex;gap:16px;align-items:center;font-size:12px;color:var(--faint);margin-top:4px;flex-wrap:wrap}
.legend .t{display:inline-block;transform:scale(.75);vertical-align:middle}
.empty{color:var(--faint);font-style:italic;padding:28px 6px}
.notes{list-style:none;margin:0;padding:0}
.notes li{padding:9px 2px;border-bottom:1px solid var(--rule);display:flex;gap:12px}
.notes li .when{color:var(--faint);font-size:12px;white-space:nowrap;font-family:var(--mono)}
.kv{display:grid;grid-template-columns:110px 1fr;gap:4px 10px;font-size:13.5px}
.kv dt{color:var(--faint)} .kv dd{margin:0;word-break:break-all}
.phase{display:inline-block;font-size:11px;letter-spacing:.14em;text-transform:uppercase;padding:2px 8px;border-radius:10px;border:1px solid var(--rule-2);color:var(--dim)}
.phase.grace,.phase.snapshotting,.phase.verifying,.phase.purging{border-color:var(--honey-lo);color:var(--honey-hi)}
.phase.deleted,.phase.aborted{border-color:#7a2a1c;color:#ffb7a6}
.phase.kept,.phase.restored{border-color:#3f6a2a;color:#cfe8b8}
.count{font-family:var(--serif);font-size:34px;color:var(--honey-hi)}
.bar{height:6px;background:var(--wax);border-radius:3px;overflow:hidden;margin:8px 0}
.bar i{display:block;height:100%;background:var(--honey)}
.hits{list-style:none;padding:0;margin:0}
.hits li{padding:8px 0;border-bottom:1px solid var(--rule)}
.hits li a.h{font-family:var(--mono);font-size:13px}
.prose{max-width:74ch} .prose p{color:var(--dim)} .prose h3{margin-top:26px}
footer{margin-top:40px;padding-top:14px;border-top:1px solid var(--rule);color:var(--faint);font-size:12.5px}
"#;

/// The page shell.
pub fn page(title: &str, csrf: Option<&str>, content: &Markup) -> Markup {
    html! {
        (PreEscaped("<!DOCTYPE html>"))
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " — Apiary" }
                @if let Some(token) = csrf {
                    meta name="csrf-token" content=(token);
                }
                style { (PreEscaped(STYLE)) }
                script src=(HTMX_JS_PATH) defer {}
                script src=(HTMX_CSRF_JS_PATH) defer {}
                script src=(HTMX_SSE_JS_PATH) defer {}
                script src="/apiary.js" defer {}
            }
            body {
                div class="wrap" {
                    header class="top" {
                        h1 { a href="/" { "Apiary" } }
                        span class="tag" { "one SQLite hive per tenant" }
                        nav {
                            a href="/" { "The comb" }
                            a href="/hive" { "Your hive" }
                            a href="/search" { "Search every hive" }
                            a href="/about" { "How it works" }
                        }
                    }
                    (content)
                    footer {
                        "Every cell is one tenant's own SQLite file, opened when somebody needs it and "
                        "closed when the fleet needs the room. Built on autumn-web 0.9's database fleet "
                        "and autumn-harvest-sqlite. "
                        a href="/about" { "How it works" } "."
                    }
                }
            }
        }
    }
}

/// Thousands-separated integer.
#[must_use]
pub fn commas(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A byte count for people.
#[must_use]
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = n;
    let mut unit = 0;
    while v >= 1024 && unit + 1 < UNITS.len() {
        v /= 1024;
        unit += 1;
    }
    format!("{} {}", commas(v), UNITS[unit])
}

/// A stat tile.
pub fn stat(value: impl maud::Render, label: &str, lit: bool) -> Markup {
    html! {
        div class=(if lit { "stat lit" } else { "stat" }) {
            b { (value) }
            span { (label) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commas_group_thousands() {
        assert_eq!(commas(0), "0");
        assert_eq!(commas(999), "999");
        assert_eq!(commas(1_000), "1,000");
        assert_eq!(commas(1_234_567), "1,234,567");
    }

    #[test]
    fn bytes_scale() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(20_480), "20 KiB");
        assert_eq!(bytes(5 * 1024 * 1024 * 1024), "5 GiB");
    }

    #[test]
    fn the_page_loads_the_three_htmx_scripts_and_ours() {
        let html = page("x", Some("tok"), &html! { p { "hi" } }).into_string();
        for src in [
            HTMX_JS_PATH,
            HTMX_CSRF_JS_PATH,
            HTMX_SSE_JS_PATH,
            "/apiary.js",
        ] {
            assert!(html.contains(src), "{src}");
        }
        assert!(html.contains(r#"name="csrf-token" content="tok""#));
    }
}
