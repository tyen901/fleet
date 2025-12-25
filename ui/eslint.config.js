import tsParser from "@typescript-eslint/parser";
import tsPlugin from "@typescript-eslint/eslint-plugin";
import reactPlugin from "eslint-plugin-react";
import reactHooksPlugin from "eslint-plugin-react-hooks";

const recommendedRules = {
  ...tsPlugin.configs.recommended.rules,
  ...reactPlugin.configs.recommended.rules,
  ...reactHooksPlugin.configs.recommended.rules,
};

export default {
  files: ["**/*.{js,jsx,ts,tsx}"],
  ignores: ["node_modules/**", "dist/**", "build/**"],
  languageOptions: {
    parser: tsParser,
    sourceType: "module",
    parserOptions: {
      ecmaVersion: "latest",
      ecmaFeatures: { jsx: true },
    },
  },
  plugins: {
    "@typescript-eslint": tsPlugin,
    react: reactPlugin,
    "react-hooks": reactHooksPlugin,
  },
  settings: {
    react: { version: "detect" },
  },
  rules: {
    ...recommendedRules,
    "react/react-in-jsx-scope": "off",
    "react/prop-types": "off",
    "react-hooks/set-state-in-effect": "off",
    "@typescript-eslint/no-unused-vars": [
      "warn",
      {
        argsIgnorePattern: "^_",
        varsIgnorePattern: "^_",
      },
    ],
  },
};
