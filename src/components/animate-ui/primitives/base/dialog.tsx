"use client";

import * as React from "react";
import { Dialog as DialogPrimitive } from "@base-ui/react/dialog";
import { AnimatePresence, motion, type HTMLMotionProps } from "motion/react";

import { useControlledState } from "@/hooks/use-controlled-state";

type DialogContextType = {
  isOpen: boolean;
  setIsOpen: DialogProps["onOpenChange"];
};

const DialogContext = React.createContext<DialogContextType | null>(null);

function DialogProvider({
  value,
  children,
}: {
  value: DialogContextType;
  children?: React.ReactNode;
}) {
  return <DialogContext.Provider value={value}>{children}</DialogContext.Provider>;
}

function useDialog() {
  const ctx = React.useContext(DialogContext);
  if (!ctx) throw new Error("useDialog must be used within a Dialog");
  return ctx;
}

function useDialogOptional() {
  return React.useContext(DialogContext);
}

type DialogProps = React.ComponentProps<typeof DialogPrimitive.Root>;

function Dialog(props: DialogProps) {
  const [isOpen, setIsOpen] = useControlledState({
    value: props?.open,
    defaultValue: props?.defaultOpen,
    onChange: props?.onOpenChange,
  });

  return (
    <DialogProvider value={{ isOpen, setIsOpen }}>
      <DialogPrimitive.Root data-slot="dialog" {...props} onOpenChange={setIsOpen} />
    </DialogProvider>
  );
}

type DialogTriggerProps = React.ComponentProps<typeof DialogPrimitive.Trigger>;

function DialogTrigger(props: DialogTriggerProps) {
  return <DialogPrimitive.Trigger data-slot="dialog-trigger" {...props} />;
}

type DialogPortalProps = Omit<React.ComponentProps<typeof DialogPrimitive.Portal>, "keepMounted">;

function DialogPortal(props: DialogPortalProps) {
  const ctx = useDialogOptional();

  if (!ctx) {
    return <DialogPrimitive.Portal data-slot="dialog-portal" {...props} />;
  }

  return (
    <AnimatePresence>
      {ctx.isOpen && <DialogPrimitive.Portal data-slot="dialog-portal" keepMounted {...props} />}
    </AnimatePresence>
  );
}

type DialogBackdropProps = Omit<React.ComponentProps<typeof DialogPrimitive.Backdrop>, "render"> &
  HTMLMotionProps<"div">;

function DialogBackdrop({
  transition = { duration: 0.2, ease: "easeInOut" },
  ...props
}: DialogBackdropProps) {
  return (
    <DialogPrimitive.Backdrop
      data-slot="dialog-backdrop"
      render={
        <motion.div
          key="dialog-backdrop"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={transition}
          {...props}
        />
      }
    />
  );
}

type DialogPopupProps = Omit<React.ComponentProps<typeof DialogPrimitive.Popup>, "render"> &
  HTMLMotionProps<"div">;

function DialogPopup({
  initialFocus,
  finalFocus,
  transition = { type: "spring", stiffness: 400, damping: 30 },
  ...props
}: DialogPopupProps) {
  return (
    <DialogPrimitive.Popup
      initialFocus={initialFocus}
      finalFocus={finalFocus}
      render={
        <motion.div
          key="dialog-popup"
          data-slot="dialog-popup"
          initial={{ opacity: 0, scale: 0.95, y: 8 }}
          animate={{ opacity: 1, scale: 1, y: 0 }}
          exit={{ opacity: 0, scale: 0.95, y: 8 }}
          transition={transition}
          {...props}
        />
      }
    />
  );
}

type DialogCloseProps = React.ComponentProps<typeof DialogPrimitive.Close>;

function DialogClose(props: DialogCloseProps) {
  return <DialogPrimitive.Close data-slot="dialog-close" {...props} />;
}

type DialogTitleProps = React.ComponentProps<typeof DialogPrimitive.Title>;

function DialogTitle(props: DialogTitleProps) {
  return <DialogPrimitive.Title data-slot="dialog-title" {...props} />;
}

type DialogDescriptionProps = React.ComponentProps<typeof DialogPrimitive.Description>;

function DialogDescription(props: DialogDescriptionProps) {
  return <DialogPrimitive.Description data-slot="dialog-description" {...props} />;
}

export {
  Dialog,
  DialogPortal,
  DialogBackdrop,
  DialogClose,
  DialogTrigger,
  DialogPopup,
  DialogTitle,
  DialogDescription,
  useDialog,
  type DialogProps,
  type DialogTriggerProps,
  type DialogPortalProps,
  type DialogCloseProps,
  type DialogBackdropProps,
  type DialogPopupProps,
  type DialogTitleProps,
  type DialogDescriptionProps,
  type DialogContextType,
};
