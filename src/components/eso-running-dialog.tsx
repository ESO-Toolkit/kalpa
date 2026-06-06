import { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";

interface EsoRunningDialogProps {
  open: boolean;
  /** Called when the user chooses to update anyway. `dontAskAgain` reflects the checkbox. */
  onConfirm: (dontAskAgain: boolean) => void;
  /** Called when the user cancels or dismisses the dialog. */
  onCancel: () => void;
}

export function EsoRunningDialog({ open, onConfirm, onCancel }: EsoRunningDialogProps) {
  const [dontAskAgain, setDontAskAgain] = useState(false);

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onCancel()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>ESO is running</DialogTitle>
          <DialogDescription>
            Addon files will be updated on disk, but Elder Scrolls Online won&apos;t see the changes
            until you type <code>/reloadui</code> in chat or relog.
          </DialogDescription>
        </DialogHeader>

        <label className="flex cursor-pointer items-center gap-2 px-1 text-[13px] text-muted-foreground/90">
          <Checkbox checked={dontAskAgain} onCheckedChange={(next) => setDontAskAgain(!!next)} />
          Don&apos;t show this again
        </label>

        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            Cancel
          </Button>
          <Button onClick={() => onConfirm(dontAskAgain)}>Update anyway</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
