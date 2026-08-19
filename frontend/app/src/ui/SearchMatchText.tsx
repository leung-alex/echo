import type { ReactNode } from "react";

const asciiOnly = /^[\x00-\x7f]*$/;

function graphemes(text: string): string[] {
  if (asciiOnly.test(text)) return text.split("");
  return Array.from(text);
}

export function getSearchMatchIndices(text: string, query: string): number[] {
  const characters = graphemes(text);
  const folded = characters.map((character) => character.toLocaleLowerCase());
  const matched = new Set<number>();
  for (const term of query
    .trim()
    .toLocaleLowerCase()
    .split(/\s+/)
    .filter(Boolean)) {
    const needle = graphemes(term);
    for (let start = 0; start <= folded.length - needle.length; start += 1) {
      if (
        needle.every(
          (character, offset) => folded[start + offset] === character,
        )
      ) {
        for (let offset = 0; offset < needle.length; offset += 1)
          matched.add(start + offset);
      }
    }
  }
  return [...matched].sort((left, right) => left - right);
}

export function SearchMatchText({
  text,
  indices,
}: {
  text: string;
  indices: readonly number[];
}): ReactNode {
  const characters = graphemes(text);
  const matched = new Set(
    indices.filter((index) => index >= 0 && index < characters.length),
  );
  if (matched.size === 0) return text;
  const parts: Array<{ matched: boolean; text: string }> = [];
  for (const [index, character] of characters.entries()) {
    const isMatch = matched.has(index);
    const current = parts.at(-1);
    if (current?.matched === isMatch) current.text += character;
    else parts.push({ matched: isMatch, text: character });
  }
  return parts.map((part, index) =>
    part.matched ? (
      <mark className="echo-search-match" key={index}>
        {part.text}
      </mark>
    ) : (
      part.text
    ),
  );
}
