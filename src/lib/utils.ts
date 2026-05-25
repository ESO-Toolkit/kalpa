import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function formatRelativeDate(iso: string): string {
  const now = Date.now();
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "";
  const seconds = Math.floor((now - then) / 1000);
  if (seconds < 0) return "Today";
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} minute${minutes !== 1 ? "s" : ""} ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hour${hours !== 1 ? "s" : ""} ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days} day${days !== 1 ? "s" : ""} ago`;
  const months = Math.floor(days / 30);
  if (months < 12) return `${months} month${months !== 1 ? "s" : ""} ago`;
  const years = Math.floor(months / 12);
  return `${years} year${years !== 1 ? "s" : ""} ago`;
}

export function formatRelativeExpiry(iso: string): string {
  const now = Date.now();
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "";
  const seconds = Math.floor((then - now) / 1000);
  if (seconds <= 0) return "expired";
  const minutes = Math.floor(seconds / 60);
  if (minutes === 0) return "Expires in less than a minute";
  if (minutes < 60) return `Expires in ~${minutes} minute${minutes !== 1 ? "s" : ""}`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `Expires in ~${hours} hour${hours !== 1 ? "s" : ""}`;
  const days = Math.floor(hours / 24);
  return `Expires in ~${days} day${days !== 1 ? "s" : ""}`;
}

const HTML_ENTITIES: Record<string, string> = {
  amp: "&",
  lt: "<",
  gt: ">",
  quot: '"',
  "#39": "'",
  apos: "'",
  nbsp: " ",
  ndash: "–",
  mdash: "—",
  hellip: "…",
  laquo: "«",
  raquo: "»",
  lsquo: "‘",
  rsquo: "’",
  ldquo: "“",
  rdquo: "”",
  bull: "•",
  middot: "·",
  copy: "©",
  reg: "®",
  trade: "™",
  deg: "°",
  times: "×",
  divide: "÷",
  plusmn: "±",
  micro: "µ",
  eacute: "é",
  egrave: "è",
  agrave: "à",
  aacute: "á",
  uuml: "ü",
  ouml: "ö",
  auml: "ä",
  iuml: "ï",
  ccedil: "ç",
  ntilde: "ñ",
  szlig: "ß",
  oslash: "ø",
  aring: "å",
  aelig: "æ",
  Eacute: "É",
  Agrave: "À",
  Aacute: "Á",
  Uuml: "Ü",
  Ouml: "Ö",
  Auml: "Ä",
};

export function decodeHtml(str: string): string {
  return str.replace(/&(#\d+|#x[0-9a-fA-F]+|\w+);/g, (match, entity) => {
    if (entity.startsWith("#x")) return String.fromCharCode(parseInt(entity.slice(2), 16));
    if (entity.startsWith("#")) return String.fromCharCode(Number(entity.slice(1)));
    return HTML_ENTITIES[entity] ?? match;
  });
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}
