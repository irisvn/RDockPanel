// Release notes (Markdown) shown as React elements, never as HTML: lines,
// `- ` lists (one level, wrapped lines joined), `**bold**`, `` `code` `` and
// `[text](https://…)` links. Anything else stays text.
import type { ReactNode } from "react";

/** One line's inline parts. */
function inline(text: string, key: string): ReactNode[] {
  const out: ReactNode[] = [];
  const pattern = /\*\*([^*]+)\*\*|`([^`]+)`|\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g;
  let at = 0;
  let n = 0;
  for (const m of text.matchAll(pattern)) {
    if (m.index > at) out.push(text.slice(at, m.index));
    const k = `${key}-${n++}`;
    if (m[1] !== undefined) out.push(<strong key={k}>{m[1]}</strong>);
    else if (m[2] !== undefined) out.push(<code key={k}>{m[2]}</code>);
    else
      out.push(
        <a key={k} href={m[4]} target="_blank" rel="noreferrer">
          {m[3]}
        </a>,
      );
    at = m.index + m[0].length;
  }
  if (at < text.length) out.push(text.slice(at));
  return out;
}

/** Markdown as paragraphs and lists. */
export function Markdown(props: { text: string }) {
  const blocks: ReactNode[] = [];
  let list: string[] = [];
  let paragraph: string[] = [];
  const flush = () => {
    if (paragraph.length) {
      const k = `p${blocks.length}`;
      blocks.push(<p key={k}>{inline(paragraph.join(" "), k)}</p>);
      paragraph = [];
    }
    if (list.length) {
      const k = `l${blocks.length}`;
      blocks.push(
        <ul key={k}>
          {list.map((item, i) => (
            <li key={i}>{inline(item, `${k}-${i}`)}</li>
          ))}
        </ul>,
      );
      list = [];
    }
  };
  for (const raw of props.text.split("\n")) {
    const line = raw.trimEnd();
    if (!line.trim()) {
      flush();
    } else if (/^\s*[-*] /.test(line)) {
      if (paragraph.length) flush();
      list.push(line.replace(/^\s*[-*] /, ""));
    } else if (/^#{1,6} /.test(line)) {
      flush();
      const k = `h${blocks.length}`;
      blocks.push(<p key={k}><strong>{inline(line.replace(/^#{1,6} /, ""), k)}</strong></p>);
    } else if (list.length && /^\s+/.test(raw)) {
      // A list item wrapped on the next line.
      list[list.length - 1] += ` ${line.trim()}`;
    } else {
      if (list.length) flush();
      paragraph.push(line.trim());
    }
  }
  flush();
  return <div className="rdp2-markdown">{blocks}</div>;
}
