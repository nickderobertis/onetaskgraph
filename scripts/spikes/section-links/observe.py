#!/usr/bin/env python3
"""Open every observable demonstration link in a real Chromium and record where it lands.

Reads `manifest.json` from the harness's output directory and writes `observations.json`
beside it. Each link is observed twice where it lives on GitHub:

  direct  a fresh tab loads the link's URL, as a click from the report (or a pasted link) does
  click   the source page is loaded and the link clicked in it, as a reader of the source does;
          when the page navigates in place and keeps the source page's scroll offset, that is
          recorded as such, so a heading that happens to sit there is not counted as landing

A link *lands* when, a few seconds after the page settles, the heading it names (by position,
so the repeated `## Budgets` is told from the first) sits at the top of the viewport. Anything
else is recorded as what happened instead: the top of the page, another heading, a 404, or no
navigation at all. The DOM facts behind each result — the element (if any) whose `id` or `name`
is the fragment, with and without `user-content-` — are recorded with it.

Linear is not observed: its web app needs a logged-in session this harness does not hold.
"""

from __future__ import annotations

import argparse
import glob
import json
import pathlib
import time

from playwright.sync_api import sync_playwright

MEASURE = """(names) => {
  const all = [...document.querySelectorAll('h1,h2,h3,h4,h5,h6')];
  const hs = all.filter(h => names.includes(h.textContent.trim()));
  const frag = decodeURIComponent(location.hash.slice(1));
  const describe = (e) => e ? {tag: e.tagName.toLowerCase(), id: e.id || null, name: e.getAttribute('name'), top: Math.round(e.getBoundingClientRect().top)} : null;
  return {
    url: location.href,
    title: document.title,
    scrollY: Math.round(window.scrollY),
    viewport: window.innerHeight,
    headings: hs.map(h => ({text: h.textContent.trim(), tag: h.tagName.toLowerCase(), id: h.id || null, top: Math.round(h.getBoundingClientRect().top)})),
    fragment: frag || null,
    byId: frag ? describe(document.getElementById(frag)) : null,
    byName: frag ? describe(document.getElementsByName(frag)[0]) : null,
    byUserContentId: frag ? describe(document.getElementById('user-content-' + frag)) : null,
    notOpened: document.getElementById('not-opened') ? document.getElementById('reason').textContent : null,
  };
}"""

NAMES = ["What", "Budgets", "Budget demo-quota-seconds", "Step 2.b: retry (once)"]


def verdict(link: dict, seen: dict) -> str:
    if seen.get("keptSourceScroll"):
        # GitHub's in-app (client-side) navigation between issues keeps the source page's
        # scroll offset and never scrolls to the fragment; whatever sits at the top then is
        # where the link sat in the source, not where the link points.
        return f"stays at the source page's scroll offset (y={seen['scrollY']}): in-app navigation, no scroll to the fragment"
    if seen.get("notOpened"):
        return f"opens nothing: {seen['notOpened']}"
    if "Page not found" in (seen.get("title") or "") or "404" in (seen.get("title") or ""):
        return "404: the link names no file"
    heads = seen["headings"]
    if len(heads) != 5:
        return f"not a target page ({len(heads)} of the 5 headings found; title {seen.get('title')!r})"
    if seen["scrollY"] < 40:
        # Not scrolled: a heading near the top of an unscrolled page is not a landing.
        return "stays at the top of the page"
    at_top = [i for i, h in enumerate(heads) if -60 <= h["top"] <= 200]
    if at_top:
        landed = at_top[0]
        if landed == link["heading_index"]:
            return "LANDS on the named heading"
        return f"lands on the wrong heading: heading {landed + 1} instead of {link['heading_index'] + 1}"
    return f"scrolled to y={seen['scrollY']} with no heading at the top"


def settle(page) -> None:
    try:
        page.wait_for_load_state("networkidle", timeout=15000)
    except Exception:
        pass
    time.sleep(2.5)


def observe_direct(context, link: dict) -> dict:
    page = context.new_page()
    try:
        response = page.goto(link["href"], wait_until="load", timeout=45000)
        settle(page)
        seen = page.evaluate(MEASURE, NAMES)
        seen["status"] = response.status if response else None
    finally:
        page.close()
    return seen


def observe_click(context, source_url: str, link: dict) -> dict:
    page = context.new_page()
    try:
        page.goto(source_url, wait_until="load", timeout=45000)
        settle(page)
        anchor = page.locator("a", has_text=link["code"] + " ").first
        anchor.scroll_into_view_if_needed(timeout=15000)
        href = anchor.evaluate("a => a.href")
        source_path = page.evaluate("location.pathname")
        source_scroll = page.evaluate("Math.round(scrollY)")
        before = set(context.pages)
        anchor.click()
        time.sleep(1)
        opened = [candidate for candidate in context.pages if candidate not in before]
        target = opened[0] if opened else page
        settle(target)
        seen = target.evaluate(MEASURE, NAMES)
        seen["clicked_href"] = href
        seen["sourceScrollY"] = source_scroll
        seen["keptSourceScroll"] = (
            target is page
            and source_scroll > 0
            and seen["scrollY"] == source_scroll
            and page.evaluate("location.pathname") != source_path
        )
        if target is not page:
            target.close()
    finally:
        page.close()
    return seen


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", required=True)
    parser.add_argument("--only", nargs="*", help="surfaces to observe: github github_nav board fileview vscode")
    parser.add_argument("--reclassify", action="store_true", help="recompute every verdict from the recorded measurements, opening no browser")
    args = parser.parse_args()
    out = pathlib.Path(args.out).resolve()
    manifest = json.loads((out / "manifest.json").read_text())
    obs_path = out / "observations.json"
    observations = json.loads(obs_path.read_text()) if obs_path.exists() else {}
    if args.reclassify:
        for surface in observations.values():
            for entry in surface["results"]:
                for mode in ("direct", "click"):
                    seen = entry.get(mode)
                    if seen and "headings" in seen:
                        seen["verdict"] = verdict({"heading_index": int(entry["code"][-1]) - 1}, seen)
        obs_path.write_text(json.dumps(observations, indent=2) + "\n")
        print(f"observations: {obs_path}")
        return

    surfaces = []
    if "github" in manifest:
        surfaces.append(("github", manifest["github"]["source"]["url"], manifest["github"]["links"], ("direct", "click")))
    if "github_nav" in manifest:
        surfaces.append(("github_nav", manifest["github_nav"]["source"]["url"], manifest["github_nav"]["links"], ("direct", "click")))
    if "board" in manifest:
        surfaces.append(("board", manifest["board"]["document"]["url"], manifest["board"]["links"], ("direct", "click")))
    if "local" in manifest:
        source = manifest["local"]["uploaded"]["source.md"]
        links = [dict(link, href=None) for link in manifest["local"]["links"]]
        surfaces.append(("fileview", source, links, ("click",)))
    if "vscode" in manifest:
        source = pathlib.Path(manifest["vscode"]["html"], "source.html").as_uri()
        surfaces.append(("vscode", source, manifest["local"]["links"], ("click",)))

    exe = sorted(glob.glob(str(pathlib.Path.home() / ".cache/ms-playwright/chromium-*/chrome-linux*/chrome")))[-1]
    with sync_playwright() as p:
        browser = p.chromium.launch(executable_path=exe)
        context = browser.new_context(viewport={"width": 1280, "height": 800})
        for name, source_url, links, modes in surfaces:
            if args.only and name not in args.only:
                continue
            print(f"{name}: {len(links)} links from {source_url}")
            results = []
            for link in links:
                entry = {"code": link["code"], "heading": link["heading"], "form": link["form"], "variant": link["variant"], "href": link.get("href")}
                for mode in modes:
                    try:
                        if mode == "direct":
                            if not link.get("href"):
                                entry["direct"] = {"verdict": "no link: the copied body holds none for this code"}
                                continue
                            seen = observe_direct(context, link)
                        else:
                            seen = observe_click(context, source_url, link)
                        seen["verdict"] = verdict(link, seen)
                    except Exception as error:  # a failed observation is a result, not a crash
                        seen = {"verdict": f"observation failed: {error.__class__.__name__}: {str(error).splitlines()[0]}"}
                    entry[mode] = seen
                    print(f"  {link['code']:<9} {mode:<6} {seen['verdict']}")
                results.append(entry)
            observations[name] = {"source": source_url, "browser": f"Chromium {browser.version}", "results": results}
            obs_path.write_text(json.dumps(observations, indent=2) + "\n")
        browser.close()
    print(f"observations: {obs_path}")


if __name__ == "__main__":
    main()
