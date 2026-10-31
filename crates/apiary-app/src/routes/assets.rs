//! The one script, served from its own route: Autumn's CSP is
//! `script-src 'self'`, so neither an inline `<script>` nor `hx-on` runs.

use autumn_web::prelude::*;

/// The script: grace countdowns, and clearing a form after htmx posts it.
pub const SCRIPT: &str = r"
(function () {
  function pad(n) { return n < 10 ? '0' + n : '' + n; }
  function tick() {
    var now = Date.now();
    document.querySelectorAll('[data-until]').forEach(function (el) {
      var left = Math.max(0, Math.round((+el.getAttribute('data-until') - now) / 1000));
      el.textContent = left >= 60 ? Math.floor(left / 60) + 'm ' + pad(left % 60) + 's' : left + 's';
    });
  }
  setInterval(tick, 250);
  document.addEventListener('htmx:afterSwap', tick);
  document.addEventListener('htmx:afterRequest', function (e) {
    var form = e.target;
    if (form && form.matches && form.matches('form[data-reset]') && e.detail.successful) {
      form.reset();
    }
  });
  window.apiary = { tick: tick };
})();
";

/// `/apiary.js`.
#[get("/apiary.js")]
#[public]
pub async fn script() -> impl IntoResponse {
    (
        [
            ("content-type", "text/javascript; charset=utf-8"),
            ("cache-control", "public, max-age=300"),
        ],
        SCRIPT,
    )
}

#[cfg(test)]
mod tests {
    use super::SCRIPT;

    #[test]
    fn the_script_has_no_eval_and_exposes_tick() {
        assert!(!SCRIPT.contains("eval("));
        assert!(!SCRIPT.contains("new Function"));
        assert!(SCRIPT.contains("window.apiary"));
        assert!(SCRIPT.contains("data-until"));
    }
}
