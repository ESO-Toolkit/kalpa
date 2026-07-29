import { Toaster as Sonner, type ToasterProps } from "sonner";

const Toaster = ({ ...props }: ToasterProps) => {
  return (
    <Sonner
      theme="dark"
      className="toaster group"
      toastOptions={{
        className:
          "!bg-surface-overlay !border-structure-08 !text-foreground !backdrop-blur-2xl !shadow-[0_16px_48px_var(--scrim-50),0_0_0_1px_var(--structure-03),inset_0_1px_0_var(--structure-06)]",
      }}
      style={
        {
          "--normal-bg": "var(--surface-overlay)",
          "--normal-text": "var(--foreground)",
          "--normal-border": "var(--structure-08)",
          "--border-radius": "0.875rem",
        } as React.CSSProperties
      }
      {...props}
    />
  );
};

export { Toaster };
