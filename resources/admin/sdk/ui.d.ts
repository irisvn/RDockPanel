// The panel's UI kit (`ui` module version 1). Props follow the panel's own
// components; they accept the usual HTML attributes of their element.
import type * as React from "react";

type Props<E extends keyof React.JSX.IntrinsicElements> = React.ComponentProps<E> & { className?: string };

export declare const Alert: React.FC<Props<"div"> & { variant?: "default" | "destructive" | "warning" }>;
export declare const Badge: React.FC<Props<"span">>;
export declare const Button: React.FC<
  Props<"button"> & { variant?: "default" | "outline" | "ghost" | "destructive" | "secondary" | "link"; size?: "default" | "sm" | "lg" | "icon" }
>;
export declare function buttonVariants(options?: { variant?: string; size?: string; className?: string }): string;
export declare const Card: React.FC<Props<"div">>;
export declare const CardContent: React.FC<Props<"div">>;
export declare const CardHeader: React.FC<Props<"div">>;
export declare const CardTitle: React.FC<Props<"h3">>;
export declare const Checkbox: React.FC<Omit<Props<"input">, "type" | "ref"> & { indeterminate?: boolean }>;
export declare const Switch: React.FC<Omit<Props<"button">, "onChange"> & { checked: boolean; onCheckedChange: (checked: boolean) => void }>;
export declare const Dialog: React.FC<{ open: boolean; onClose: () => void; title: React.ReactNode; children?: React.ReactNode; className?: string }>;
export declare const Input: React.FC<Props<"input">>;
export declare const Label: React.FC<Props<"label">>;
export declare const NativeSelect: React.FC<Props<"select">>;
export declare const Textarea: React.FC<Props<"textarea">>;
export declare const Page: React.FC<Props<"div">>;
export declare const PageHeader: React.FC<{ title: React.ReactNode; description?: React.ReactNode; actions?: React.ReactNode; eyebrow?: React.ReactNode }>;
export declare const Loading: React.FC<{ label?: string }>;
export declare const ErrorState: React.FC<{ error: unknown }>;
export declare function cn(...inputs: Array<string | false | null | undefined>): string;
