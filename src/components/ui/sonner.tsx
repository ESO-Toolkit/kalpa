import { Toaster as Sonner, type ToasterProps } from "sonner";

const Toaster = ({ ...props }: ToasterProps) => {
  return (
    <Sonner
      theme="dark"
      className="toaster group"
      toastOptions={{
        className:
          "!bg-[rgba(12,20,38,0.96)] !border-white/[0.08] !text-foreground !backdrop-blur-2xl !shadow-[0_16px_48px_rgba(0,0,0,0.5),0_0_0_1px_rgba(255,255,255,0.03),inset_0_1px_0_rgba(255,255,255,0.06)]",
      }}
      style={
        {
          "--normal-bg": "rgba(12, 20, 38, 0.96)",
          "--normal-text": "var(--foreground)",
          "--normal-border": "rgba(255, 255, 255, 0.08)",
          "--border-radius": "0.875rem",
        } as React.CSSProperties
      }
      {...props}
    />
  );
};

export { Toaster };
