import type { Rule } from './types';

/** youpinSkuId 是否与其它规则重复。value 为空 → 不算重复。editingId 排除自身。 */
export function isDuplicateYoupin(rules: Rule[], value: string, editingId?: string): boolean {
  const v = (value ?? '').trim();
  if (!v) return false;
  return rules.some((r) => r.id !== editingId && (r.youpin_sku_id ?? '').trim() === v);
}
