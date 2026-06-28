import { useEffect, useState } from 'react';
import {
  Button,
  Input,
  Modal,
  ModalContent,
  ModalHeader,
  ModalTitle,
  ModalFooter,
} from '@talon-ui/react';
import { isDuplicateYoupin } from '../lib/ruleDedup';
import type { Rule } from '../lib/types';

interface Props {
  open: boolean;
  rules: Rule[];
  editing?: Rule;
  onClose: () => void;
  onSubmit: (rule: Rule) => void;
}

function numOrNull(v: string): number | null {
  return v.trim() === '' ? null : Number(v);
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-tp-1">
      <span className="text-xs text-text-secondary">{label}</span>
      {children}
    </label>
  );
}

export function RuleFormModal({ open, rules, editing, onClose, onSubmit }: Props) {
  const [label, setLabel] = useState('');
  const [youpin, setYoupin] = useState('');
  const [priceMin, setPriceMin] = useState('');
  const [priceMax, setPriceMax] = useState('');
  const [qty, setQty] = useState('1');
  const [dupError, setDupError] = useState('');

  // 每次 modal 打开时重置表单
  useEffect(() => {
    if (open) {
      setLabel(editing?.label ?? '');
      setYoupin(editing?.youpin_sku_id ?? '');
      setPriceMin(editing?.price_min != null ? String(editing.price_min) : '');
      setPriceMax(editing?.price_max != null ? String(editing.price_max) : '');
      setQty(String(editing?.qty ?? 1));
      setDupError('');
    }
  }, [open, editing]);

  function handleSubmit() {
    if (isDuplicateYoupin(rules, youpin, editing?.id)) {
      setDupError('该 youpinSkuId 已存在,请勿重复添加');
      return;
    }
    const rule: Rule = {
      id: editing?.id ?? `r${Date.now()}`,
      label,
      youpin_sku_id: youpin.trim() || null,
      price_min: numOrNull(priceMin),
      price_max: numOrNull(priceMax),
      qty: Math.max(1, Number(qty) || 1),
      used: editing?.used ?? 0,
      enabled: editing?.enabled ?? true,
    };
    onSubmit(rule);
  }

  return (
    <Modal open={open} onOpenChange={(o) => !o && onClose()}>
      <ModalContent className="w-[400px] max-w-[400px]">
        <ModalHeader>
          <ModalTitle>{editing ? '编辑规则' : '新增规则'}</ModalTitle>
        </ModalHeader>

        <div className="flex flex-col gap-tp-4 px-tp-1">
          <Field label="备注名">
            <Input
              className="selectable"
              placeholder="如 苹果17"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
            />
          </Field>

          <Field label="youpinSkuId">
            <Input
              className="selectable"
              placeholder="精确匹配,如 100012345678"
              value={youpin}
              onChange={(e) => {
                setYoupin(e.target.value);
                setDupError('');
              }}
            />
            {dupError && (
              <span className="text-xs text-danger-500">{dupError}</span>
            )}
          </Field>

          <div className="grid grid-cols-2 gap-tp-3">
            <Field label="最低价(空=不限)">
              <Input
                className="selectable"
                inputMode="decimal"
                placeholder="不限"
                value={priceMin}
                onChange={(e) => setPriceMin(e.target.value)}
              />
            </Field>
            <Field label="最高价(空=不限)">
              <Input
                className="selectable"
                inputMode="decimal"
                placeholder="不限"
                value={priceMax}
                onChange={(e) => setPriceMax(e.target.value)}
              />
            </Field>
          </div>

          <Field label="数量(配额)">
            <Input
              className="selectable"
              inputMode="numeric"
              placeholder="1"
              value={qty}
              onChange={(e) => setQty(e.target.value)}
            />
          </Field>
        </div>

        <ModalFooter>
          <Button variant="ghost" onClick={onClose}>
            取消
          </Button>
          <Button variant="primary" onClick={handleSubmit}>
            确定
          </Button>
        </ModalFooter>
      </ModalContent>
    </Modal>
  );
}
