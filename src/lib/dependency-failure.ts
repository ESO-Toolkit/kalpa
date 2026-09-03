import { toast } from "sonner";

/**
 * Report dependencies that failed after the primary addon install/update
 * succeeded. Entries are backend-authored diagnostics, so keep each one intact:
 * reserved-folder refusals include the rejected destination path users need in
 * order to understand and fix the archive.
 */
export function reportDependencyFailures(failedDeps: readonly string[]): void {
  if (failedDeps.length === 0) return;

  toast.error(
    failedDeps.length === 1
      ? "A dependency could not be installed"
      : `${failedDeps.length} dependencies could not be installed`,
    { description: failedDeps.join("; ") }
  );
}
