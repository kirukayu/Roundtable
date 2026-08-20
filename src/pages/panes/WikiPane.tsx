import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useRef, useState } from "react";

import { Icon } from "../../components/Icons";
import { EASE } from "../../components/Motion";
import { Blank, Card, useToast } from "../../components/ui";
import { api } from "../../lib/ipc";
import type { WikiPage, WikiSearchResult } from "../../lib/types";

/** How many titles to render at once. The rest arrive as the list is scrolled. */
const PAGE = 120;

/**
 * The flag the wiki labels each language tab with.
 *
 * Item descriptions are written out in eleven languages behind a tabber, and
 * the tab is labelled with a flag and nothing else. Knowing which flag is which
 * is what lets the article open on the language the reader actually speaks.
 */
const FLAGS: Record<string, string> = {
  ru: "🇷🇺",
  ja: "🇯🇵",
  zh: "🇨🇳",
  de: "🇩🇪",
  fr: "🇫🇷",
  it: "🇮🇹",
  ko: "🇰🇷",
  pl: "🇵🇱",
  pt: "🇧🇷",
  es: "🇪🇸",
};

/**
 * Shows one panel of a tab group.
 *
 * The wiki's markup already marks the panel it considers current, and the
 * stylesheet hides the rest, so this only has to move that mark. No listener is
 * attached to anything inside the article: React replaces the article's markup
 * more than once for a single page, and anything bound to a child is thrown
 * away when it does — which is exactly how the first attempt at this failed,
 * silently, with every language panel still on screen.
 */
function showTab(tabber: HTMLElement, chosen: number) {
  const tabs = tabber.querySelectorAll<HTMLElement>(":scope > .wds-tabs__wrapper .wds-tabs__tab");
  const panels = tabber.querySelectorAll<HTMLElement>(":scope > .wds-tab__content");
  if (tabs.length < 2 || tabs.length !== panels.length) return;
  tabs.forEach((tab, index) => tab.classList.toggle("wds-is-current", index === chosen));
  panels.forEach((panel, index) => panel.classList.toggle("wds-is-current", index === chosen));
}

/**
 * Opens each tab group on the reader's own language, where it has one.
 *
 * A nicety rather than the mechanism: if this never runs, the wiki's own choice
 * stands and the article is still readable, in English. That is why the working
 * parts of tabbing are the stylesheet and the click handler on the container,
 * neither of which can be lost to a re-render.
 */
function preferMyLanguage(root: HTMLElement) {
  const wanted = FLAGS[navigator.language.slice(0, 2).toLowerCase()];
  if (!wanted) return;
  root.querySelectorAll<HTMLElement>(".wds-tabber").forEach((tabber) => {
    const tabs = [...tabber.querySelectorAll<HTMLElement>(":scope > .wds-tabs__wrapper .wds-tabs__tab")];
    const mine = tabs.findIndex((tab) => (tab.dataset.hash ?? "").includes(wanted));
    if (mine >= 0) showTab(tabber, mine);
  });
}

/**
 * The wiki, whole.
 *
 * Every article on the left, in order, scrolling; the article itself on the
 * right, exactly as the wiki wrote it. It opens on the wiki's own front page,
 * so there is something to read before you have searched for anything, and the
 * search field filters the list rather than dropping a menu over it.
 *
 * Links between articles are intercepted, so following them stays in here.
 */
export default function WikiPane({ edition }: { edition: string | null }) {
  const toast = useToast();
  const [query, setQuery] = useState("");
  const [source, setSource] = useState<string | null>(null);
  const [list, setList] = useState<WikiSearchResult | null>(null);
  const [page, setPage] = useState<WikiPage | null>(null);
  const [loading, setLoading] = useState(false);
  const [shown, setShown] = useState(PAGE);
  const [trail, setTrail] = useState<string[]>([]);
  const [toc, setToc] = useState<{ id: string; text: string; level: number }[]>([]);
  const article = useRef<HTMLDivElement>(null);
  const top = useRef<HTMLDivElement>(null);
  const poll = useRef<number | null>(null);

  const wiki = source ?? list?.source.id ?? null;

  const refresh = useCallback(async () => {
    try {
      // The whole index, not a page of it: the sidebar is the wiki's contents.
      setList(await api.wiki(query, edition, source ?? undefined, 5000));
    } catch {
      /* an index that is not there yet is a state, not an error */
    }
  }, [query, edition, source]);

  useEffect(() => {
    const timer = window.setTimeout(() => void refresh(), 150);
    return () => window.clearTimeout(timer);
  }, [refresh]);

  useEffect(() => setShown(PAGE), [query, source]);

  const load = useCallback(
    async (title: string, push = true) => {
      setLoading(true);
      try {
        const fetched = await api.wikiPage(title, edition, wiki ?? undefined);
        setPage(fetched);
        if (push) {
          setTrail((was) => [...was.filter((t) => t !== fetched.title), fetched.title]);
          // Only when a link was followed. Scrolling on the automatic first
          // load would hide the tabs before the user has touched anything.
          top.current?.scrollIntoView({ behavior: "smooth", block: "start" });
        }
      } catch (error) {
        toast.error("Could not open", error instanceof Error ? error.message : String(error));
      } finally {
        setLoading(false);
      }
    },
    [edition, wiki, toast],
  );

  // Open on the wiki's front page. Landing on a blank pane makes a mirrored
  // wiki look empty when it holds thousands of articles.
  const opened = useRef<string | null>(null);
  useEffect(() => {
    if (!list || list.state.titles === 0) return;
    if (opened.current === list.source.id) return;
    opened.current = list.source.id;
    setTrail([]);
    void load("Main Page", false);
  }, [list, load]);

  // Changing edition changes which wiki is authoritative.
  useEffect(() => {
    setSource(null);
    setPage(null);
    setTrail([]);
    opened.current = null;
  }, [edition]);

  /**
   * Reads the article's headings once it is in the document.
   *
   * A callback ref, not an effect: with `AnimatePresence` the element has not
   * mounted at the point an effect on `page` would run.
   */
  const mountArticle = useCallback((node: HTMLDivElement | null) => {
    article.current = node;
    if (!node) {
      setToc([]);
      return;
    }
    preferMyLanguage(node);

    const found = [...node.querySelectorAll("h1, h2, h3")]
      // Infobox field names are headings in Fandom's markup; nine of them at
      // the top of a contents list bury the real sections.
      .filter((heading) => !heading.closest(".portable-infobox, .infobox, .pi-item"))
      .map((heading, index) => {
        if (!heading.id) heading.id = `sec-${index}`;
        return {
          id: heading.id,
          text: heading.textContent?.trim() ?? "",
          level: heading.tagName === "H3" ? 3 : 2,
        };
      });
    setToc(found.filter((entry) => entry.text.length > 0 && entry.text.length < 70));
  }, []);

  /**
   * Picks the reader's language again once the article has settled.
   *
   * The mount callback fires while React is still working — for one article it
   * writes the body's markup a second time afterwards, throwing away whatever
   * the callback had touched. Running once more after the frame is painted
   * catches the final markup. Nothing depends on this succeeding; the tabs work
   * without it.
   */
  useEffect(() => {
    if (!page) return;
    const frame = requestAnimationFrame(() => {
      if (article.current) preferMyLanguage(article.current);
    });
    return () => cancelAnimationFrame(frame);
  }, [page]);

  const onBodyClick = (event: React.MouseEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement;

    // A tab label. Handled here on the container rather than on the label
    // itself, so switching keeps working after React rewrites the article.
    const tab = target.closest<HTMLElement>(".wds-tabs__tab");
    const tabber = tab?.closest<HTMLElement>(".wds-tabber");
    if (tab && tabber) {
      event.preventDefault();
      const all = [...tabber.querySelectorAll<HTMLElement>(":scope > .wds-tabs__wrapper .wds-tabs__tab")];
      showTab(tabber, all.indexOf(tab));
      return;
    }

    const anchor = target.closest("a");
    if (!anchor) return;
    const href = anchor.getAttribute("href") ?? "";
    if (!href.startsWith("#wiki:")) return;
    event.preventDefault();
    const title = href.slice(href.indexOf(":", 6) + 1);
    if (title) void load(title);
  };

  const back = () => {
    if (trail.length < 2) return;
    const next = trail.slice(0, -1);
    setTrail(next);
    void load(next[next.length - 1], false);
  };

  const index = async () => {
    try {
      await api.wikiSync(wiki ?? undefined, edition);
      poll.current = window.setInterval(async () => {
        const fresh = await api.wiki(query, edition, source ?? undefined, 5000);
        setList(fresh);
        if (!fresh.state.syncing) {
          if (poll.current) window.clearInterval(poll.current);
          poll.current = null;
          if (fresh.state.error) toast.error("Indexing failed", fresh.state.error);
          else toast.success("Wiki mirrored", `${fresh.state.titles} articles`);
        }
      }, 1000);
    } catch (error) {
      toast.error("Could not start", error instanceof Error ? error.message : String(error));
    }
  };

  useEffect(() => () => {
    if (poll.current) window.clearInterval(poll.current);
  }, []);

  const state = list?.state;

  if (state && state.titles === 0) {
    return (
      <Card>
        <Blank
          icon={Icon.Library}
          title={`${list?.source.name} wiki is not mirrored yet`}
          action={
            <button type="button" className="btn btn--solid" onClick={index} disabled={state.syncing}>
              {state.syncing ? <span className="spin" /> : null}
              {state.syncing ? state.message : "Mirror it"}
            </button>
          }
        >
          Every article title, fetched once. Pages download as you open them and
          stay readable offline.
        </Blank>
      </Card>
    );
  }

  const titles = list?.titles ?? [];
  const visible = titles.slice(0, shown);

  return (
    <div className="wk" ref={top}>
      <aside className="wk__side">
        {list && list.sources.length > 1 && (
          <div className="wk__sources">
            {list.sources.map((entry) => (
              <button
                key={entry.id}
                type="button"
                className="codex__filter"
                data-on={wiki === entry.id}
                onClick={() => {
                  setSource(entry.id);
                  setQuery("");
                  opened.current = null;
                }}
              >
                {entry.name}
              </button>
            ))}
          </div>
        )}

        <div className="wk__find">
          <Icon.Search size={14} />
          <input
            className="wk__input"
            placeholder="Filter articles"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && titles[0]) void load(titles[0]);
              if (event.key === "Escape") setQuery("");
            }}
          />
          {query && (
            <button type="button" className="wk__clear" onClick={() => setQuery("")}>
              <Icon.Close size={13} />
            </button>
          )}
        </div>

        {/* The contents of the wiki, in order, all of it. */}
        <div
          className="wk__list"
          onScroll={(event) => {
            const el = event.currentTarget;
            if (el.scrollTop + el.clientHeight > el.scrollHeight - 400) {
              setShown((was) => Math.min(was + PAGE, titles.length));
            }
          }}
        >
          {visible.map((title) => (
            <button
              key={title}
              type="button"
              className="wk__item"
              data-on={page?.title === title}
              onClick={() => void load(title)}
            >
              {title}
            </button>
          ))}
          {visible.length === 0 && (
            <p className="w4" style={{ fontSize: "var(--t-xs)", padding: "var(--s4)" }}>
              Nothing matches that.
            </p>
          )}
        </div>

        <div className="between w4" style={{ fontSize: "var(--t-2xs)" }}>
          <span>
            {query ? `${titles.length} of ${state?.titles ?? 0}` : `${state?.titles ?? 0} articles`}
          </span>
          <span>{state?.cachedPages ?? 0} read</span>
        </div>
      </aside>

      <section className="wk__stage">
        <AnimatePresence mode="wait">
          {loading && !page ? (
            <motion.div key="loading" className="wk__loading" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
              <span className="spin" />
            </motion.div>
          ) : page ? (
            <motion.article
              key={page.title}
              className="wk__article"
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.4, ease: EASE }}
            >
              <div className="wk__head">
                <div className="row" style={{ gap: "var(--s3)", minWidth: 0 }}>
                  {trail.length > 1 && (
                    <button type="button" className="btn btn--ghost btn--icon btn--sm" onClick={back} title="Back">
                      <Icon.Back size={14} />
                    </button>
                  )}
                  <h2 className="wk__h">{page.title}</h2>
                </div>
                <a className="btn btn--ghost btn--sm" href={page.origin} target="_blank" rel="noreferrer noopener">
                  Original
                </a>
              </div>

              {/*
                Already stripped on the backend of every script, inline handler
                and javascript: href, with article links rewritten to the form
                this pane intercepts.
              */}
              <div
                className="wiki__body"
                ref={mountArticle}
                onClick={onBodyClick}
                dangerouslySetInnerHTML={{ __html: page.html }}
              />
            </motion.article>
          ) : (
            <motion.div key="blank" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
              <Blank icon={Icon.Library} title="Pick an article">
                {state?.titles ?? 0} of them, on the left.
              </Blank>
            </motion.div>
          )}
        </AnimatePresence>
      </section>

      {page && toc.length > 1 && (
        <nav className="wk__toc">
          {toc.map((entry) => (
            <button
              key={entry.id}
              type="button"
              className="wk__toc-item"
              data-level={entry.level}
              onClick={() =>
                article.current
                  ?.querySelector(`#${CSS.escape(entry.id)}`)
                  ?.scrollIntoView({ behavior: "smooth", block: "start" })
              }
            >
              {entry.text}
            </button>
          ))}
        </nav>
      )}
    </div>
  );
}
