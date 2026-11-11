/** 가격/사이즈/시간 포맷 헬퍼.
 *  Polymarket 스타일에 맞춰 — 소수점 4자리 가격, 콤마 구분 size.
 */

export const fmt$ = (n: number | null | undefined, d = 2) => {
  if (n == null || Number.isNaN(n)) return "—";
  const sign = n < 0 ? "-" : "";
  return sign + "$" + Math.abs(n).toLocaleString("en-US", { minimumFractionDigits: d, maximumFractionDigits: d });
};
export const fmtN = (n: number | null | undefined, d = 0) => {
  if (n == null || Number.isNaN(n)) return "—";
  return n.toLocaleString("en-US", { minimumFractionDigits: d, maximumFractionDigits: d });
};
export const fmtPct = (n: number | null | undefined, d = 1) => {
  if (n == null || Number.isNaN(n)) return "—";
  return n.toFixed(d) + "%";
};
export const fmtPx = (n: number | null | undefined) => {
  if (n == null) return "—";
  return n.toFixed(4);
};
export const fmtTime = (ts: string | Date) => {
  const d = typeof ts === "string" ? new Date(ts) : ts;
  return d.toISOString().slice(11, 19);
};
export const fmtDate = (ts: string | Date) => {
  const d = typeof ts === "string" ? new Date(ts) : ts;
  return d.toISOString().slice(0, 10);
};
export const fmtDateTime = (ts: string | Date) => {
  const d = typeof ts === "string" ? new Date(ts) : ts;
  return d.toISOString().slice(0, 19).replace("T", " ") + "Z";
};
export const relTime = (ts: string | Date) => {
  const d = typeof ts === "string" ? new Date(ts) : ts;
  const sec = (Date.now() - d.getTime()) / 1000;
  if (sec < 60)      return Math.floor(sec) + "s 전";
  if (sec < 3600)    return Math.floor(sec/60) + "m 전";
  if (sec < 86400)   return Math.floor(sec/3600) + "h 전";
  return Math.floor(sec/86400) + "d 전";
};
export const cls = (v: number | null | undefined) =>
  v == null ? "" : v > 0 ? "text-buy" : v < 0 ? "text-sell" : "";
export const cn = (...x: (string | false | null | undefined)[]) =>
  x.filter(Boolean).join(" ");
