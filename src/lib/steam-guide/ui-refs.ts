/**
 * The names of interface elements, marked in the guide's texts (`[[Magasin]]`): cut into segments
 * the page sets as text, never HTML. An orphan `[[`, a lone `]]` or `[[]]` stays plain text: a typo
 * shows its brackets rather than swallowing the rest of the sentence.
 */
export interface UiSegment {
  text: string;
  ref: boolean;
}

export function splitUiRefs(text: string): UiSegment[] {
  const out: UiSegment[] = [];
  const plain = (s: string) => {
    if (s.length === 0) return;
    const last = out[out.length - 1];
    if (last && !last.ref) last.text += s;
    else out.push({ text: s, ref: false });
  };
  let i = 0;
  while (i < text.length) {
    const open = text.indexOf("[[", i);
    if (open < 0) {
      plain(text.slice(i));
      break;
    }
    const close = text.indexOf("]]", open + 2);
    const name = close < 0 ? "" : text.slice(open + 2, close);
    if (close < 0 || name.length === 0 || name.includes("[[") || name.trim() !== name) {
      plain(text.slice(i, open + 2));
      i = open + 2;
      continue;
    }
    plain(text.slice(i, open));
    out.push({ text: name, ref: true });
    i = close + 2;
  }
  return out;
}
