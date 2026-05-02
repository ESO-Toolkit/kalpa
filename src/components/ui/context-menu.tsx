import { useEffect, useRef, useState, useCallback } from "react";
import { createPortal } from "react-dom";
import { motion, useReducedMotion } from "motion/react";
import { cn } from "@/lib/utils";

export interface ContextMenuItem {
  label: string;
  icon?: React.ElementType;
  onClick: () => void;
  disabled?: boolean;
  destructive?: boolean;
}

export interface ContextMenuSeparator {
  separator: true;
}

export type ContextMenuEntry = ContextMenuItem | ContextMenuSeparator;

function isSeparator(entry: ContextMenuEntry): entry is ContextMenuSeparator {
  return "separator" in entry;
}

interface ContextMenuProps {
  items: ContextMenuEntry[];
  position: { x: number; y: number };
  onClose: () => void;
}

export function ContextMenu({ items, position, onClose }: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [activeIndex, setActiveIndex] = useState(-1);
  const prefersReducedMotion = useReducedMotion();

  const actionItems = items
    .map((item, i) => (isSeparator(item) ? null : { item, index: i }))
    .filter(Boolean) as { item: ContextMenuItem; index: number }[];

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
        return;
      }

      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActiveIndex((prev) => {
          const currentPos = actionItems.findIndex((a) => a.index === prev);
          const next = currentPos < actionItems.length - 1 ? currentPos + 1 : 0;
          return actionItems[next].index;
        });
      }

      if (e.key === "ArrowUp") {
        e.preventDefault();
        setActiveIndex((prev) => {
          const currentPos = actionItems.findIndex((a) => a.index === prev);
          const next = currentPos > 0 ? currentPos - 1 : actionItems.length - 1;
          return actionItems[next].index;
        });
      }

      if (e.key === "Enter") {
        e.preventDefault();
        const active = actionItems.find((a) => a.index === activeIndex);
        if (active && !active.item.disabled) {
          active.item.onClick();
          onClose();
        }
      }
    },
    [actionItems, activeIndex, onClose]
  );

  useEffect(() => {
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [onClose]);

  // Viewport edge clamping
  const [adjustedPos, setAdjustedPos] = useState(position);
  useEffect(() => {
    if (!menuRef.current) return;
    const rect = menuRef.current.getBoundingClientRect();
    let { x, y } = position;
    if (x + rect.width > window.innerWidth - 8) x = window.innerWidth - rect.width - 8;
    if (y + rect.height > window.innerHeight - 8) y = window.innerHeight - rect.height - 8;
    if (x < 8) x = 8;
    if (y < 8) y = 8;
    setAdjustedPos({ x, y });
  }, [position]);

  return createPortal(
    <motion.div
      ref={menuRef}
      role="menu"
      aria-label="Addon actions"
      initial={prefersReducedMotion ? { opacity: 1 } : { opacity: 0, scale: 0.95 }}
      animate={{ opacity: 1, scale: 1 }}
      transition={prefersReducedMotion ? { duration: 0 } : { duration: 0.1, ease: "easeOut" }}
      className="fixed z-[9998] min-w-[180px] rounded-xl border border-white/[0.08] bg-[rgba(10,18,36,0.95)] backdrop-blur-xl p-1 shadow-[0_8px_32px_rgba(0,0,0,0.5),0_1px_0_rgba(255,255,255,0.04)_inset]"
      style={{ left: adjustedPos.x, top: adjustedPos.y }}
    >
      {items.map((entry, i) => {
        if (isSeparator(entry)) {
          return <div key={`sep-${i}`} className="my-1 border-t border-white/[0.06]" />;
        }

        const Icon = entry.icon;
        return (
          <button
            key={i}
            role="menuitem"
            disabled={entry.disabled}
            className={cn(
              "flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-xs font-medium transition-colors outline-none",
              entry.destructive
                ? "text-red-400 hover:bg-red-500/10"
                : "text-foreground/80 hover:bg-white/[0.06] hover:text-foreground",
              entry.disabled && "opacity-40 pointer-events-none",
              activeIndex === i && (entry.destructive ? "bg-red-500/10" : "bg-white/[0.06]")
            )}
            onClick={() => {
              entry.onClick();
              onClose();
            }}
            onMouseEnter={() => setActiveIndex(i)}
          >
            {Icon && <Icon className="size-3.5 shrink-0" />}
            {entry.label}
          </button>
        );
      })}
    </motion.div>,
    document.body
  );
}
