/** 展示层的格式化工具。 */

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const exponent = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** exponent;
  return `${value.toFixed(exponent === 0 ? 0 : 1)} ${units[exponent]}`;
}

export function formatDate(unixSeconds: number | null | undefined): string {
  if (!unixSeconds) return "—";
  return new Date(unixSeconds * 1000).toLocaleString("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function formatDimensions(width: number | null, height: number | null): string {
  if (!width || !height) return "未知尺寸";
  const megapixels = (width * height) / 1_000_000;
  return `${width} × ${height} · ${megapixels.toFixed(1)} MP`;
}
