import { writable, derived } from 'svelte/store';
import zh from './locales/zh';
import en from './locales/en';

export type Locale = 'zh' | 'en';
export type TranslationKey = keyof typeof zh;

export type TranslationParams = Record<string, string | number | boolean | null | undefined>;

const translations: Record<Locale, Record<string, string>> = { zh, en };

export const localeNames: Record<Locale, string> = {
  zh: '中文',
  en: 'English',
};

export const availableLocales = Object.keys(localeNames) as Locale[];

export const locale = writable<Locale>('zh');

function interpolate(template: string, params?: TranslationParams): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (_match, key: string) => {
    if (!Object.prototype.hasOwnProperty.call(params, key)) return `{${key}}`;
    const value = params[key];
    return value === null || value === undefined ? '' : String(value);
  });
}

export const t = derived(locale, ($locale) => {
  const dict = translations[$locale] || translations.zh;
  return (key: TranslationKey | string, params?: TranslationParams): string => {
    const raw = dict[key as string] || (key as string);
    return interpolate(raw, params);
  };
});
