#!/usr/bin/env python3
"""Follow one link clicked inside a GitHub page over time: does the page ever scroll to it?

    uv run --no-project --with playwright==1.64.1 python probe_click.py <source page URL> <link code>

Loads the source page, clicks the link whose text starts with the code (for example `GI-I-U3`),
then prints the page's scroll offset and the fragment element's position every half second for
twelve seconds, and once more after a reload. On a GitHub issue the offset stays the source
page's own: the in-app navigation never scrolls to the fragment.
"""
import glob, pathlib, sys, time
from playwright.sync_api import sync_playwright
src, code = sys.argv[1], sys.argv[2]
exe = sorted(glob.glob(str(pathlib.Path.home() / ".cache/ms-playwright/chromium-*/chrome-linux*/chrome")))[-1]
with sync_playwright() as p:
    b = p.chromium.launch(executable_path=exe); c = b.new_context(viewport={"width":1280,"height":800}); pg = c.new_page()
    pg.goto(src, wait_until="load"); pg.wait_for_load_state("networkidle"); time.sleep(2)
    a = pg.locator("a", has_text=code + " ").first; a.scroll_into_view_if_needed()
    print("source scrollY before click", pg.evaluate("scrollY"), "link top", a.evaluate("e=>Math.round(e.getBoundingClientRect().top)"))
    a.click(); t0 = time.time()
    for _ in range(24):
        time.sleep(0.5)
        frag = pg.evaluate("decodeURIComponent(location.hash.slice(1))")
        print(f"{time.time()-t0:4.1f}s", pg.evaluate("location.pathname"), "scrollY", pg.evaluate("Math.round(scrollY)"),
              "anchor top", pg.evaluate("f=>{const e=document.getElementById(f)||document.getElementsByName(f)[0];return e?Math.round(e.getBoundingClientRect().top):null}", frag))
    pg.reload(); time.sleep(4)
    print("after reload: scrollY", pg.evaluate("Math.round(scrollY)"))
    b.close()
