/**
 * 展示层数值工具：金额/费率/数量一律保持字符串渲染（Rust Decimal 直出），
 * 仅做纯字符串尾零裁剪，不转浮点、不做四舍五入、不改变精度语义。
 */
export function trimDecimal(value: unknown): string {
  const str = String(value ?? '—')
  if (!str.includes('.')) return str
  return str.replace(/0+$/, '').replace(/\.$/, '')
}