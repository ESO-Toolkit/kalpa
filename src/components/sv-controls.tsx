import { useEffect, useRef, useState } from "react";
import type { EffectiveField, SvTreeNode } from "../types";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { MinusIcon, PlusIcon } from "lucide-react";
import { motion } from "motion/react";

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
      } ${checked ? "bg-primary" : "bg-white/[0.12]"}`}
      aria-label={field.label}
    >
      <motion.span
        layout
        transition={{ type: "spring", stiffness: 500, damping: 30 }}
        className={`inline-block size-3.5 rounded-full bg-white shadow ${
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

  const step = field.props.step ?? 1;
  const { min, max } = field.props;

  const clamp = (v: number) => {
    if (min !== undefined && v < min) return min;
    if (max !== undefined && v > max) return max;
    return v;
  };

  const commit = () => {
    // Treat empty/whitespace input as invalid (Number("") === 0) and revert.
    if (localValue.trim() === "") {
      setLocalValue(fieldVal);
      return;
    }
    const num = Number(localValue);
    if (isNaN(num)) {
      setLocalValue(fieldVal);
      return;
    }
    // Clamp typed values to min/max, matching the ± buttons.
    const clamped = clamp(num);
    // Only commit when the value actually changed to avoid a spurious dirty flag.
    if (clamped === Number(field.value ?? 0)) {
      setLocalValue(fieldVal);
      return;
    }
    onChange(clamped);
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
        className="w-20 rounded-lg border border-white/[0.08] bg-white/[0.04] px-2 py-1 text-xs text-foreground outline-none focus:border-accent-sky/50 focus:ring-1 focus:ring-accent-sky/30"
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
  const fieldValue = Number(field.value) || min;

  // Track the value locally during the drag for instant visual feedback, and only
  // fire the heavy onChange (which rebuilds the SV tree + re-renders the editor) on
  // release. Mirrors the local-state + commit pattern used by NumberControl.
  const [localValue, setLocalValue] = useState(fieldValue);
  const [prevFieldValue, setPrevFieldValue] = useState(fieldValue);
  if (prevFieldValue !== fieldValue) {
    setPrevFieldValue(fieldValue);
    setLocalValue(fieldValue);
  }

  const commit = () => {
    if (localValue !== fieldValue) onChange(localValue);
  };

  return (
    <div className="flex items-center gap-2">
      <input
        type="range"
        className="h-1.5 flex-1 cursor-pointer appearance-none rounded-full bg-white/[0.1] accent-primary"
        min={min}
        max={max}
        step={step}
        value={localValue}
        onChange={(e) => setLocalValue(Number(e.target.value))}
        onPointerUp={commit}
        onKeyUp={commit}
        onBlur={commit}
        disabled={field.readOnly}
      />
      <span className="w-10 text-right text-xs text-muted-foreground tabular-nums">
        {localValue}
      </span>
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

  // Preview the picked color locally while the OS picker is open, and batch the
  // r/g/b(/a) update into a single onChangeColor commit fired on the native
  // `change` event (when the picker is closed) — the previous code fired 3-4
  // onChangeColor calls per pointer-move, each rebuilding the SV tree.
  const [localHex, setLocalHex] = useState(hexColor);
  const [prevHex, setPrevHex] = useState(hexColor);
  if (prevHex !== hexColor) {
    setPrevHex(hexColor);
    setLocalHex(hexColor);
  }

  const inputRef = useRef<HTMLInputElement>(null);

  // Commit on the native `change` event (fired once when the OS picker closes),
  // not on React's onChange (which fires continuously as the user drags in the
  // picker). Re-subscribes only when the committed color / alpha / handler change
  // — during a drag only `localHex` changes, so the listener stays put.
  useEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    const commit = () => {
      const next = el.value;
      if (next === hexColor) return;
      const nr = parseInt(next.slice(1, 3), 16) / 255;
      const ng = parseInt(next.slice(3, 5), 16) / 255;
      const nb = parseInt(next.slice(5, 7), 16) / 255;
      onChangeColor(nr, ng, nb, a);
    };
    el.addEventListener("change", commit);
    return () => el.removeEventListener("change", commit);
  }, [hexColor, a, onChangeColor]);

  return (
    <div className="flex items-center gap-2">
      <input
        ref={inputRef}
        type="color"
        value={localHex}
        onChange={(e) => setLocalHex(e.target.value)}
        className="size-7 cursor-pointer rounded border border-white/[0.1] bg-transparent p-0"
        disabled={field.readOnly}
      />
      <span className="text-xs text-muted-foreground font-mono">{localHex}</span>
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

  const commit = () => {
    if (localValue !== fieldVal) onChange(localValue);
  };

  if (field.props.multiline) {
    return (
      <textarea
        className="w-full rounded-lg border border-white/[0.08] bg-white/[0.04] px-2 py-1.5 text-xs text-foreground outline-none focus:border-accent-sky/50 focus:ring-1 focus:ring-accent-sky/30 resize-y"
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
      className="w-full max-w-xs rounded-lg border border-white/[0.08] bg-white/[0.04] px-2 py-1 text-xs text-foreground outline-none focus:border-accent-sky/50 focus:ring-1 focus:ring-accent-sky/30"
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
  onChange: (val: string | number) => void;
}) {
  const options = field.props.options ?? [];
  const currentVal = String(field.value ?? "");
  const allOptions = options.includes(currentVal) ? options : [currentVal, ...options];

  const commit = (v: string | null) => {
    if (!v) return;
    // Preserve numeric type when the field is currently a number and the
    // chosen option parses cleanly as a finite number.
    if (typeof field.value === "number") {
      const num = Number(v);
      if (v.trim() !== "" && Number.isFinite(num)) {
        onChange(num);
        return;
      }
    }
    onChange(v);
  };

  return (
    <Select value={currentVal} onValueChange={commit} disabled={field.readOnly}>
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
      className="w-full rounded-lg border border-white/[0.08] bg-white/[0.04] px-2 py-1.5 font-mono text-xs text-foreground outline-none focus:border-accent-sky/50 focus:ring-1 focus:ring-accent-sky/30 resize-y"
      rows={2}
      value={localValue}
      onChange={(e) => setLocalValue(e.target.value)}
      onBlur={() => {
        if (localValue !== fieldVal) onChange(localValue);
      }}
      disabled={field.readOnly}
    />
  );
}
