/* Soyuz site: theme toggle, scroll reveal, hero tilt, keyboard palette. No tracking. */
(function () {
  "use strict";
  var root = document.documentElement;
  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)");
  var prefersLight = window.matchMedia("(prefers-color-scheme: light)");

  /* Theme */
  var toggle = document.querySelector(".theme-toggle");
  function current() {
    var t = root.getAttribute("data-theme");
    return t || (prefersLight.matches ? "light" : "dark");
  }
  function labelToggle() {
    var next = current() === "dark" ? "light" : "dark";
    toggle.setAttribute("aria-label", "Switch to " + next + " theme");
  }
  function setTheme(t) {
    root.setAttribute("data-theme", t);
    try { localStorage.setItem("soyuz-theme", t); } catch (e) { /* storage unavailable */ }
    labelToggle();
  }
  function flipTheme() { setTheme(current() === "dark" ? "light" : "dark"); }
  toggle.hidden = false;
  labelToggle();
  toggle.addEventListener("click", flipTheme);
  if (prefersLight.addEventListener) prefersLight.addEventListener("change", labelToggle);

  /* Header border once scrolled */
  var header = document.querySelector(".top");
  var heroShot = document.querySelector(".hero-shot");
  var ticking = false;
  function onScroll() {
    if (ticking) return;
    ticking = true;
    requestAnimationFrame(function () {
      ticking = false;
      header.classList.toggle("scrolled", window.scrollY > 8);
      if (!reduce.matches && heroShot) {
        var r = heroShot.getBoundingClientRect();
        var vh = window.innerHeight || 1;
        // 0 when the shot's top sits at the bottom of the viewport, 1 once it reaches 35%.
        var p = (vh - r.top) / (vh * 0.65);
        p = Math.max(0, Math.min(1, p));
        heroShot.style.setProperty("--p", p.toFixed(3));
      }
    });
  }
  if (heroShot && !reduce.matches) heroShot.classList.add("tilt");
  window.addEventListener("scroll", onScroll, { passive: true });
  window.addEventListener("resize", onScroll);
  onScroll();

  /* Scroll reveal: elements start visible; JS hides only those still below the fold. */
  if ("IntersectionObserver" in window && !reduce.matches) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (e.isIntersecting) { e.target.classList.remove("pending"); io.unobserve(e.target); }
      });
    }, { rootMargin: "0px 0px -8% 0px", threshold: 0.08 });
    var vh0 = window.innerHeight;
    document.querySelectorAll("[data-reveal]").forEach(function (el) {
      if (el.getBoundingClientRect().top > vh0 * 0.92) {
        el.classList.add("pending");
        io.observe(el);
      }
    });
  }

  /* Palette and help dialogs */
  var pal = document.getElementById("palette");
  var help = document.getElementById("help");
  if (!pal || typeof pal.showModal !== "function") return;
  var input = document.getElementById("pal-input");
  var list = document.getElementById("pal-list");
  var lastFocus = null;

  var items = [];
  document.querySelectorAll("main section[id]").forEach(function (s) {
    var h = s.querySelector("h1, h2");
    if (!h) return;
    items.push({ label: s.id === "top" ? "Top of the page" : h.textContent.trim(), hint: "Section", run: function () { goTo(s); } });
  });
  document.querySelectorAll(".feat h3").forEach(function (h) {
    var art = h.closest(".feat");
    items.push({ label: h.textContent.trim(), hint: "Feature", run: function () { goTo(art); } });
  });
  items.push(
    { label: "Download the latest release", hint: "GitHub", run: function () { location.href = "https://github.com/ulaas/soyuz/releases/latest"; } },
    { label: "View the source on GitHub", hint: "GitHub", run: function () { location.href = "https://github.com/ulaas/soyuz"; } },
    { label: "Switch light and dark theme", hint: "T", run: flipTheme },
    { label: "Keyboard shortcuts for this page", hint: "?", run: function () { openDialog(help); } }
  );

  function goTo(el) {
    var target = el.querySelector("h1, h2, h3") || el;
    el.scrollIntoView({ behavior: reduce.matches ? "auto" : "smooth", block: "start" });
    if (!target.hasAttribute("tabindex")) target.setAttribute("tabindex", "-1");
    target.focus({ preventScroll: true });
  }

  var shown = [], sel = 0;
  function render() {
    var q = input.value.trim().toLowerCase();
    shown = !q ? items.slice() : items.filter(function (it) { return it.label.toLowerCase().indexOf(q) !== -1; });
    if (q && !shown.length) {
      // No substring hit: fall back to a loose in-order letter match.
      shown = items.filter(function (it) {
        var l = it.label.toLowerCase(), i = 0;
        for (var c = 0; c < l.length && i < q.length; c++) if (l[c] === q[i]) i++;
        return i === q.length;
      });
    }
    if (sel >= shown.length) sel = 0;
    list.textContent = "";
    if (!shown.length) {
      var li0 = document.createElement("li");
      li0.className = "empty";
      li0.setAttribute("role", "option");
      li0.setAttribute("aria-disabled", "true");
      li0.textContent = "No match. Try “features” or “download”.";
      list.appendChild(li0);
      input.removeAttribute("aria-activedescendant");
      return;
    }
    shown.forEach(function (it, i) {
      var li = document.createElement("li");
      li.id = "pal-opt-" + i;
      li.setAttribute("role", "option");
      li.setAttribute("aria-selected", i === sel ? "true" : "false");
      var a = document.createElement("span");
      a.textContent = it.label;
      var b = document.createElement("small");
      b.textContent = it.hint;
      li.appendChild(a);
      li.appendChild(b);
      li.addEventListener("mousemove", function () { if (sel !== i) { sel = i; mark(); } });
      li.addEventListener("click", function () { choose(i); });
      list.appendChild(li);
    });
    mark();
  }
  function mark() {
    Array.prototype.forEach.call(list.children, function (li, i) {
      li.setAttribute("aria-selected", i === sel ? "true" : "false");
    });
    var cur = document.getElementById("pal-opt-" + sel);
    if (cur) { input.setAttribute("aria-activedescendant", cur.id); cur.scrollIntoView({ block: "nearest" }); }
  }
  function choose(i) {
    var it = shown[i];
    if (!it) return;
    pal.close();
    it.run();
  }
  input.addEventListener("input", function () { sel = 0; render(); });
  input.addEventListener("keydown", function (e) {
    if (e.key === "ArrowDown") { e.preventDefault(); if (shown.length) { sel = (sel + 1) % shown.length; mark(); } }
    else if (e.key === "ArrowUp") { e.preventDefault(); if (shown.length) { sel = (sel - 1 + shown.length) % shown.length; mark(); } }
    else if (e.key === "Enter") { e.preventDefault(); choose(sel); }
  });

  function openDialog(d) {
    if (d.open) return;
    if (pal.open) pal.close();
    if (help.open) help.close();
    lastFocus = document.activeElement;
    if (d === pal) { input.value = ""; sel = 0; render(); }
    d.showModal();
    if (d === pal) input.focus();
  }
  [pal, help].forEach(function (d) {
    d.addEventListener("close", function () {
      if (lastFocus && document.contains(lastFocus) && document.activeElement === document.body) lastFocus.focus();
    });
    // Click on the backdrop closes.
    d.addEventListener("click", function (e) { if (e.target === d) d.close(); });
  });

  /* Visible keycaps in the hero and the header palette button */
  var keyBtns = {};
  document.querySelectorAll(".key-btn").forEach(function (b) {
    keyBtns[b.getAttribute("data-act")] = b;
    b.addEventListener("click", function () { act(b.getAttribute("data-act")); });
  });
  var tryRow = document.querySelector(".try");
  if (tryRow) tryRow.hidden = false;
  var palBtn = document.querySelector(".palette-open");
  palBtn.hidden = false;
  palBtn.addEventListener("click", function () { act("palette"); });

  function flash(name) {
    var b = keyBtns[name];
    if (!b) return;
    b.classList.add("pressed");
    setTimeout(function () { b.classList.remove("pressed"); }, 220);
  }
  function act(name) {
    flash(name);
    if (name === "palette") openDialog(pal);
    else if (name === "help") openDialog(help);
    else if (name === "theme") flipTheme();
  }

  var stops = Array.prototype.slice.call(document.querySelectorAll("main section[id]"));
  function step(dir) {
    var y = window.scrollY + 90, idx = 0;
    stops.forEach(function (s, i) { if (s.offsetTop <= y) idx = i; });
    var next = stops[Math.max(0, Math.min(stops.length - 1, idx + dir))];
    if (next) goTo(next);
  }

  document.addEventListener("keydown", function (e) {
    var t = e.target;
    var typing = t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
    if ((e.ctrlKey || e.metaKey) && !e.altKey && (e.key === "k" || e.key === "K")) {
      e.preventDefault();
      if (pal.open) pal.close(); else act("palette");
      return;
    }
    if (typing || e.ctrlKey || e.metaKey || e.altKey) return;
    if (pal.open || help.open) return;
    switch (e.key) {
      case "/": e.preventDefault(); act("palette"); break;
      case "?": e.preventDefault(); act("help"); break;
      case "t": case "T": act("theme"); break;
      case "j": case "J": step(1); break;
      case "k": case "K": step(-1); break;
      case "d": case "D": goTo(document.getElementById("download")); break;
      default: break;
    }
  });
})();
