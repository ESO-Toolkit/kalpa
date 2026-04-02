import { useState } from "react";
import type { EffectiveField, SvTreeNode } from "../types";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { MinusIcon, PlusIcon } from "lucide-react";

export function ToggleControl({
  field,
  onChange,
}: {
  field: EffectiveField;
  onChange: (val: boolean) => void;
}) {
  const checked = field.value === true;
  return (
    <button
      role="switch"
      aria-checked={checked}
      onClick={() => !field.readOnly && onChange(!checked)}
      className={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors ${
        field.readOnly ? "opacity-50 cursor-not-allowed" : "cursor-pointer"
      } ${checked ? "bg-[#c4a44a]" : "bg-white/[0.12]"}`}
      aria-label={field.label}
    >
      <span
        className={`inline-block size-3.5 rounded-full bg-white shadow transition-transform ${
          checked ? "translate-x-[18px]" : "translate-x-[3px]"
        }`}
      />
    </button>
  );
}

export function NumberControl({
  field,
  onChange,
}: {
  field: EffectiveField;
  onChange: (val: number) => void;
}) {
  const fieldVal = String(field.value ?? 0);
  const [localValue, setLocalValue] = useState(fieldVal);
  const [prevFieldVal, setPrevFieldVal] = useState(fieldVal);
  if (prevFieldVal !== fieldVal) {
    setPrevFieldVal(fieldVal);
    setLocalValue(fieldVal);
  }

  const commit = () => {
    const num = Number(localValue);
    if (!isNaN(num)) onChange(num);
    else setLocalValue(String(field.value ?? 0));
  };

  const step = field.props.step ?? 1;
  const { min, max } = field.props;

  const clamp = (v: number) => {
    if (min !== undefined && v < min) return min;
    if (max !== undefined && v > max) return max;
    return v;
  };

  return (
    <div className="flex items-center gap-1">
      <button
        onClick={() => onChange(clamp((Number(field.value) || 0) - step))}
        className="flex size-6 items-center justify-center rounded border border-white/[0.08] bg-white/[0.04] text-muted-foreground hover:bg-white/[0.08] hover:text-foreground"
        disabled={field.readOnly}
      >
        <MinusIcon className="size-3" />
      </button>
      <input
        type="number"
        className="w-20 rounded-lg border border-white/[0.08] bg-white/[0.04] px-2 py-1 text-xs text-foreground outline-none focus:border-[#38bdf8]/50 focus:ring-1 focus:ring-[#38bdf8]/30"
        value={localValue}
        onChange={(e) => setLocalValue(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === "Enter" && commit()}
        disabled={field.readOnly}
        step={step}
        min={min}
        max={max}
      />
      <button
        onClick={() => onChange(clamp((Number(field.value) || 0) + step))}
        className="flex size-6 items-center justify-center rounded border border-white/[0.08] bg-white/[0.04] text-muted-foreground hover:bg-white/[0.08] hover:text-foreground"
        disabled={field.readOnly}
      >
        <PlusIcon className="size-3" />
      </button>
    </div>
  );
}

export function SliderControl({
  field,
  onChange,
}: {
  field: EffectiveField;
  onChange: (val: number) => void;
}) {
  const min = field.props.min ?? 0;
  const max = field.props.max ?? 100;
  const step = field.props.step ?? 1;
  const value = Number(field.value) || min;

  return (
    <div className="flex items-center gap-2">
      <input
        type="range"
        className="h-1.5 flex-1 cursor-pointer appearance-none rounded-full bg-white/[0.1] accent-[#c4a44a]"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        disabled={field.readOnly}
      />
      <span className="w-10 text-right text-xs text-muted-foreground tabular-nums">{value}</span>
    </div>
  );
}

export function ColorControl({
  field,
  originalNode,
  onChangeColor,
}: {
  field: EffectiveField;
  originalNode: SvTreeNode | null;
  onChangeColor: (r: number, g: number, b: number, a?: number) => void;
}) {
  const children = originalNode?.children ?? [];
  const getVal = (key: string) => {
    const c = children.find((ch) => ch.key === key);
    return c ? Number(c.value ?? 0) : 0;
  };
  const r = getVal("r");
  const g = getVal("g");
  const b = getVal("b");
  const a = children.some((ch) => ch.key === "a") ? getVal("a") : undefined;

  const hexColor = `#${Math.round(r * 255)
    .toString(16)
    .padStart(2, "0")}${Math.round(g * 255)
    .toString(16)
    .padStart(2, "0")}${Math.round(b * 255)
    .toString(16)
    .padStart(2, "0")}`;

  return (
    <div className="flex items-center gap-2">
      <input
        type="color"
        value={hexColor}
        onChange={(e) => {
          const hex = e.target.value;
          const nr = parseInt(hex.slice(1, 3), 16) / 255;
          const ng = parseInt(hex.slice(3, 5), 16) / 255;
          const nb = parseInt(hex.slice(5, 7), 16) / 255;
          onChangeColor(nr, ng, nb, a);
        }}
        className="size-7 cursor-pointer rounded border border-white/[0.1] bg-transparent p-0"
        disabled={field.readOnly}
      />
      <span className="text-xs text-muted-foreground font-mono">{hexColor}</span>
      {a !== undefined && (
        <span className="text-xs text-muted-foreground/60">a: {a.toFixed(2)}</span>
      )}
    </div>
  );
}

export function TextControl({
  field,
  onChange,
}: {
  field: EffectiveField;
  onChange: (val: string) => void;
}) {
  const fieldVal = String(field.value ?? "");
  const [localValue, setLocalValue] = useState(fieldVal);
  const [prevFieldVal, setPrevFieldVal] = useState(fieldVal);
  if (prevFieldVal !== fieldVal) {
    setPrevFieldVal(fieldVal);
    setLocalValue(fieldVal);
  }

  const commit = () => onChange(localValue);

  if (field.props.multiline) {
    return (
      <textarea
        className="w-full rounded-lg border border-white/[0.08] bg-white/[0.04] px-2 py-1.5 text-xs text-foreground outline-none focus:border-[#38bdf8]/50 focus:ring-1 focus:ring-[#38bdf8]/30 resize-y"
        rows={3}
        value={localValue}
        onChange={(e) => setLocalValue(e.target.value)}
        onBlur={commit}
        disabled={field.readOnly}
      />
    );
  }

  return (
    <input
      type="text"
      className="w-full max-w-xs rounded-lg border border-white/[0.08] bg-white/[0.04] px-2 py-1 text-xs text-foreground outline-none focus:border-[#38bdf8]/50 focus:ring-1 focus:ring-[#38bdf8]/30"
      value={localValue}
      onChange={(e) => setLocalValue(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => e.key === "Enter" && commit()}
      disabled={field.readOnly}
    />
  );
}

export function DropdownControl({
  field,
  onChange,
}: {
  field: EffectiveField;
  onChange: (val: string) => void;
}) {
  const options = field.props.options ?? [];
  const currentVal = String(field.value ?? "");
  const allOptions = options.includes(currentVal) ? options : [currentVal, ...options];

  return (
    <Select value={currentVal} onValueChange={(v) => v && onChange(v)} disabled={field.readOnly}>
      <SelectTrigger className="h-7 w-40 text-xs">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {allOptions.map((opt) => (
          <SelectItem key={opt} value={opt}>
            {opt}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

export function ReadonlyControl({ field }: { field: EffectiveField }) {
  return (
    <span className="text-xs text-muted-foreground/60 italic">
      {field.value === null ? "nil" : String(field.value)}
    </span>
  );
}

export function RawControl({
  field,
  onChange,
}: {
  field: EffectiveField;
  onChange: (val: string) => void;
}) {
  const fieldVal = String(field.value ?? "");
  const [localValue, setLocalValue] = useState(fieldVal);
  const [prevFieldVal, setPrevFieldVal] = useState(fieldVal);
  if (prevFieldVal !== fieldVal) {
    setPrevFieldVal(fieldVal);
    setLocalValue(fieldVal);
  }

  return (
    <textarea
      className="w-full rounded-lg border border-white/[0.08] bg-white/[0.04] px-2 py-1.5 font-mono text-xs text-foreground outline-none focus:border-[#38bdf8]/50 focus:ring-1 focus:ring-[#38bdf8]/30 resize-y"
      rows={2}
      value={localValue}
      onChange={(e) => setLocalValue(e.target.value)}
      onBlur={() => onChange(localValue)}
      disabled={field.readOnly}
    />
  );
}
