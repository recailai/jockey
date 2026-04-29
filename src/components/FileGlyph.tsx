import { createMemo } from "solid-js";
import { Dynamic } from "solid-js/web";
import { getFileVisual } from "../lib/fileVisual";

type FileGlyphProps = {
  name: string;
  class?: string;
  iconSize?: number;
};

export default function FileGlyph(props: FileGlyphProps) {
  const visual = createMemo(() => getFileVisual(props.name));

  return (
    <span class={`file-glyph ${visual().toneClass} ${props.class ?? ""}`}>
      <Dynamic component={visual().Icon} size={props.iconSize ?? 12} class="file-glyph-icon" />
    </span>
  );
}
