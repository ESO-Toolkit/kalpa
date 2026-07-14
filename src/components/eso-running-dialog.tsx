import { memo, useState } from "react";
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

function EsoRunningDialogBase({ open, onConfirm, onCancel }: EsoRunningDialogProps) {
  const [dontAskAgain, setDontAskAgain] = useState(false);

  // The component stays mounted between prompts, so clear the opt-out as the dialog
  // closes — otherwise a previously-checked box would carry into a fresh prompt.
  const handleConfirm = () => {
    setDontAskAgain(false);
    onConfirm(dontAskAgain);
  };
  const handleCancel = () => {
    setDontAskAgain(false);
    onCancel();
  };

  return (
    <Dialog open={open} onOpenChange={(next) => !next && handleCancel()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>ESO is running</DialogTitle>
          <DialogDescription>
            Addon files will be written to disk, but Elder Scrolls Online won&apos;t see the changes
            until you type <code>/reloadui</code> in chat or relog.
          </DialogDescription>
        </DialogHeader>

        <label className="flex cursor-pointer items-center gap-2 px-1 text-[13px] text-muted-foreground/90">
          <Checkbox checked={dontAskAgain} onCheckedChange={(next) => setDontAskAgain(!!next)} />
          Don&apos;t show this again
        </label>

        <DialogFooter>
          <Button variant="outline" onClick={handleCancel}>
            Cancel
          </Button>
          <Button onClick={handleConfirm}>Update anyway</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// Memoized: props are one boolean + two stable callbacks.
export const EsoRunningDialog = memo(EsoRunningDialogBase);
