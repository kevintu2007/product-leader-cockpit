/**
 * The message runtime: typed keys, named parameters, plural choice.
 *
 * English is the canonical catalog. Its keys are the only keys, and each
 * message's parameters are read from its own `{name}` placeholders by the
 * type system, so a call that names a missing key or forgets a parameter
 * does not compile. Every other catalog must carry exactly the same keys with
 * exactly the same placeholders; `catalogs.test.ts` holds them to that.
 *
 * Messages are whole sentences with named slots. Nothing here joins
 * fragments, because word order differs between the six languages.
 */
import { EN } from "./catalogs/en";
import type { Locale } from "./locale";

export type MessageKey = keyof typeof EN;

/** The `{name}` placeholders in one message, as a union of names. */
type Placeholders<S extends string> = S extends `${string}{${infer Name}}${infer Rest}`
  ? Name | Placeholders<Rest>
  : never;

export type MessageParams<K extends MessageKey> = Readonly<
  Record<Placeholders<(typeof EN)[K]>, string | number>
>;

/** Every locale's catalog: the English keys, each with its own wording. */
export type Catalog = Readonly<Record<MessageKey, string>>;

/** A key that needs no parameters, or the parameters it needs. */
export type ParamsArgument<K extends MessageKey> =
  Placeholders<(typeof EN)[K]> extends never ? [] : [params: MessageParams<K>];

/** Plural messages are stored as `<base>.one` / `<base>.other` (and more,
 * where a language needs them); this is the set of bases. */
export type PluralBase = MessageKey extends infer K
  ? K extends `${infer Base}.other`
    ? Base
    : never
  : never;

/** The parameters a plural message takes besides the `{count}` it is given. */
type PluralPlaceholders<B extends PluralBase> = Exclude<
  Placeholders<(typeof EN)[`${B}.other` & MessageKey]>,
  "count"
>;

export type PluralParamsArgument<B extends PluralBase> =
  PluralPlaceholders<B> extends never
    ? []
    : [params: Readonly<Record<PluralPlaceholders<B>, string | number>>];

export function isMessageKey(key: string): key is MessageKey {
  return Object.hasOwn(EN, key);
}

/**
 * Fill a template's `{name}` slots. A slot with no value is left as written,
 * so a missing parameter is visible rather than silently blank. Numbers are
 * written the way the locale writes them.
 */
export function interpolate(
  locale: Locale,
  template: string,
  params: Readonly<Record<string, string | number>>,
): string {
  const numbers = new Intl.NumberFormat(locale);
  return template.replace(/\{([a-zA-Z0-9_]+)\}/g, (whole, name: string) => {
    // Own properties only: `{toString}` with no such parameter stays visible
    // rather than printing an inherited function.
    if (!Object.hasOwn(params, name)) {
      return whole;
    }
    const value = params[name];
    if (value === undefined) {
      return whole;
    }
    return typeof value === "number" ? numbers.format(value) : value;
  });
}

export interface Translator {
  readonly locale: Locale;
  <K extends MessageKey>(key: K, ...params: ParamsArgument<K>): string;
  /** The plural form `Intl.PluralRules` picks for `count`, falling back to
   * `<base>.other`. `count` is also available to the message as `{count}`. */
  plural<B extends PluralBase>(base: B, count: number, ...params: PluralParamsArgument<B>): string;
  /** A key only known at run time (a host `messageKey`), or `null` when this
   * catalog has no such message. */
  lookup(key: string, params: Readonly<Record<string, string | number>>): string | null;
}

export function createTranslator(locale: Locale, catalog: Catalog): Translator {
  const rules = new Intl.PluralRules(locale);
  const translate = <K extends MessageKey>(key: K, ...args: ParamsArgument<K>): string =>
    interpolate(locale, catalog[key], args[0] ?? {});
  const read = (key: string): string | undefined => (isMessageKey(key) ? catalog[key] : undefined);
  return Object.assign(translate, {
    locale,
    plural<B extends PluralBase>(base: B, count: number, ...args: PluralParamsArgument<B>) {
      const template = read(`${base}.${rules.select(count)}`) ?? read(`${base}.other`) ?? base;
      return interpolate(locale, template, { ...(args[0] ?? {}), count });
    },
    lookup(key: string, params: Readonly<Record<string, string | number>>) {
      const template = read(key);
      return template === undefined ? null : interpolate(locale, template, params);
    },
  });
}
