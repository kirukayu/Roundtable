export function bytes(value: number | null | undefined): string {
  if (value === null || value === undefined) return "—";
  if (value === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const exponent = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  const scaled = value / 1024 ** exponent;
  const digits = scaled >= 100 || exponent === 0 ? 0 : 1;
  return `${scaled.toFixed(digits)} ${units[exponent]}`;
}

/** Playtime as the game shows it: hours and minutes, never bare seconds. */
export function playtime(seconds: number): string {
  if (!seconds) return "—";
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (hours === 0) return `${minutes}m`;
  return `${hours}h ${minutes}m`;
}

export function when(iso: string | null | undefined): string {
  if (!iso) return "never";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "—";

  const elapsed = Date.now() - date.getTime();
  const minute = 60_000;
  const hour = 60 * minute;
  const day = 24 * hour;

  if (elapsed < minute) return "just now";
  if (elapsed < hour) return `${Math.floor(elapsed / minute)} min ago`;
  if (elapsed < day) return `${Math.floor(elapsed / hour)} h ago`;
  if (elapsed < 7 * day) return `${Math.floor(elapsed / day)} d ago`;

  return date.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export function exact(iso: string | null | undefined): string {
  if (!iso) return "—";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "—";
  return date.toLocaleString();
}

/** Keeps the start and end of a long path, dropping the middle. */
export function shortPath(path: string, max = 52): string {
  if (path.length <= max) return path;
  const head = path.slice(0, Math.ceil(max / 2) - 2);
  const tail = path.slice(-Math.floor(max / 2) + 1);
  return `${head}…${tail}`;
}

export function fileName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

export function pluralise(count: number, singular: string, plural?: string): string {
  return `${count} ${count === 1 ? singular : (plural ?? `${singular}s`)}`;
}
