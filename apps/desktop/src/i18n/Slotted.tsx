import { Fragment, type ReactNode } from "react";

/**
 * A whole catalog message some of whose slots are elements rather than text
 * -- a live timer, a muted provenance note, a bold label. The message keeps
 * its own punctuation, spacing and word order in every language; only the
 * named slots become the elements, so nothing outside a slot joins it.
 *
 * `text` is the message with its text slots already filled and its element
 * slots still written as `{name}`. A slot the message does not contain is
 * appended rather than dropped, so an element is never silently lost.
 */
export function Slotted({
  text,
  slots,
}: {
  readonly text: string;
  readonly slots: Readonly<Record<string, ReactNode>>;
}): ReactNode {
  const used = new Set<string>();
  const parts = text.split(/(\{[a-zA-Z0-9_]+\})/).map((part, index) => {
    const name = /^\{([a-zA-Z0-9_]+)\}$/.exec(part)?.[1];
    if (name !== undefined && Object.hasOwn(slots, name)) {
      used.add(name);
      return <Fragment key={String(index)}>{slots[name]}</Fragment>;
    }
    return part;
  });
  const missing = Object.keys(slots).filter((name) => !used.has(name));
  return (
    <>
      {parts}
      {missing.map((name) => (
        <Fragment key={`missing-${name}`}>{slots[name]}</Fragment>
      ))}
    </>
  );
}
