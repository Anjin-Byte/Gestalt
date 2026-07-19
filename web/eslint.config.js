// Engineering Codex Reference ESLint Config: typed linting via typescript-eslint
// flat config, strict + stylistic type-checked bundles, browser globals (this is
// the browser shell; the Node globals of the template do not apply).
import eslint from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";

// One rule set for app and test sources (flat-config blocks don't inherit
// rules across `files` patterns, so both blocks spread this).
const sharedRules = {
  // --- Promise / async hygiene (typed) ---
  "@typescript-eslint/no-floating-promises": "error",
  "@typescript-eslint/no-misused-promises": "error",
  "@typescript-eslint/require-await": "error",
  "@typescript-eslint/prefer-promise-reject-errors": "error",

  // --- Exhaustiveness ---
  "@typescript-eslint/switch-exhaustiveness-check": [
    "error",
    { considerDefaultExhaustiveForUnions: true },
  ],

  // --- Containment of any (typed) ---
  "@typescript-eslint/no-unsafe-assignment": "error",
  "@typescript-eslint/no-unsafe-call": "error",
  "@typescript-eslint/no-unsafe-member-access": "error",
  "@typescript-eslint/no-unsafe-return": "error",
  "@typescript-eslint/no-unsafe-argument": "error",
  "@typescript-eslint/no-explicit-any": "error",

  // --- Type-correct coercions / comparisons ---
  "@typescript-eslint/no-base-to-string": "error",
  "@typescript-eslint/restrict-template-expressions": [
    "error",
    { allowNumber: true, allowBoolean: true },
  ],
  "@typescript-eslint/restrict-plus-operands": "error",
  eqeqeq: ["error", "always"],

  // --- Security-adjacent ---
  "no-eval": "error",
  "no-implied-eval": "error",
  "@typescript-eslint/no-implied-eval": "error",

  // --- Style / readability ---
  "@typescript-eslint/consistent-type-imports": [
    "error",
    { prefer: "type-imports", fixStyle: "inline-type-imports" },
  ],
  "@typescript-eslint/no-unused-vars": [
    "error",
    { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
  ],
};

export default tseslint.config(
  {
    ignores: ["dist/**", "node_modules/**"],
  },

  eslint.configs.recommended,
  ...tseslint.configs.strictTypeChecked,
  ...tseslint.configs.stylisticTypeChecked,

  // vite.config.ts is outside the app tsconfig (it runs in Node); lint it
  // without type information rather than dragging Node types into the project.
  {
    files: ["vite.config.ts"],
    ...tseslint.configs.disableTypeChecked,
  },

  {
    files: ["src/**/*.ts"],
    ignores: ["src/**/*.test.ts"],
    languageOptions: {
      parserOptions: {
        project: "./tsconfig.json",
        tsconfigRootDir: import.meta.dirname,
      },
      globals: {
        ...globals.browser,
      },
    },
    rules: sharedRules,
  },

  // Test files: the same typed rules, resolved against the test project
  // (which admits the vitest import), plus one named relaxation.
  {
    files: ["src/**/*.test.ts"],
    languageOptions: {
      parserOptions: {
        project: "./tsconfig.test.json",
        tsconfigRootDir: import.meta.dirname,
      },
      globals: {
        ...globals.browser,
      },
    },
    rules: {
      ...sharedRules,
      // Asserting on `expect(fake.method)` mock references is the vitest
      // idiom; the fakes' methods are vi.fn values that carry no `this`.
      "@typescript-eslint/unbound-method": "off",
    },
  },
);
