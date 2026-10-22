// Just enough Markdown for summaries: headings, bullets, bold, paragraphs. Input is escaped first.
const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
const inline = (s: string) => esc(s).replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>").replace(/(^|\s)_(.+?)_(?=\s|$)/g, "$1<em>$2</em>");

export function markdown(src: string): string {
  const out: string[] = [];
  let list = false;
  const close = () => {
    if (list) out.push("</ul>");
    list = false;
  };
  for (const raw of src.split("\n")) {
    const line = raw.trimEnd();
    const h = line.match(/^(#{1,6})\s+(.*)$/);
    const li = line.match(/^\s*(?:[-*•]|\d+[.)])\s+(.*)$/);
    if (h) {
      close();
      out.push(`<h4>${inline(h[2])}</h4>`);
    } else if (li) {
      if (!list) out.push("<ul>");
      list = true;
      out.push(`<li>${inline(li[1])}</li>`);
    } else if (line.trim()) {
      close();
      out.push(`<p>${inline(line)}</p>`);
    } else {
      close();
    }
  }
  close();
  return out.join("");
}
