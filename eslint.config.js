import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";

export default tseslint.config(
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    plugins: {
      "react-hooks": reactHooks,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
    },
  },
  {
    ignores: [
      "dist/",
      "src-tauri/",
      "node_modules/",
      "src/components/animate-ui/",
      "src/hooks/use-auto-height.tsx",
      "src/hooks/use-is-in-view.tsx",
      "src/lib/get-strict-context.tsx",
    ],
  }
);
