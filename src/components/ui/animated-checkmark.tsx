import { motion, useReducedMotion } from "motion/react";

interface AnimatedCheckmarkProps {
  size?: number;
  color?: string;
  strokeWidth?: number;
  delay?: number;
}

export function AnimatedCheckmark({
  size = 20,
  color = "#34d399",
  strokeWidth = 2.5,
  delay = 0,
}: AnimatedCheckmarkProps) {
  const prefersReducedMotion = useReducedMotion();

  return (
    <motion.svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      initial={{ scale: prefersReducedMotion ? 1 : 0.5, opacity: 0 }}
      animate={{ scale: 1, opacity: 1 }}
      transition={
        prefersReducedMotion
          ? { duration: 0 }
          : { type: "spring", stiffness: 300, damping: 20, delay: delay / 1000 }
      }
    >
      <motion.path
        d="M5 13l4 4L19 7"
        stroke={color}
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
        initial={{ pathLength: prefersReducedMotion ? 1 : 0 }}
        animate={{ pathLength: 1 }}
        transition={
          prefersReducedMotion
            ? { duration: 0 }
            : { duration: 0.4, ease: "easeOut", delay: delay / 1000 + 0.1 }
        }
      />
    </motion.svg>
  );
}
