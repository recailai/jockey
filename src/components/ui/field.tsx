import type { JSX } from "solid-js";
import { splitProps } from "solid-js";

export type InputProps = JSX.InputHTMLAttributes<HTMLInputElement>;
export type TextareaProps = JSX.TextareaHTMLAttributes<HTMLTextAreaElement>;

export function Input(props: InputProps) {
  const [local, rest] = splitProps(props, ["class"]);
  return <input {...rest} class={`jui-field ${local.class ?? ""}`} />;
}

export function Textarea(props: TextareaProps) {
  const [local, rest] = splitProps(props, ["class"]);
  return <textarea {...rest} class={`jui-field jui-textarea ${local.class ?? ""}`} />;
}
