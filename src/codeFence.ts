export type CodeFenceEdit = {
  value: string;
  selectionStart: number;
  selectionEnd: number;
};

export function wrapCodeFence(
  value: string,
  selectionStart: number,
  selectionEnd: number,
): CodeFenceEdit {
  const safeStart = Math.max(0, Math.min(value.length, selectionStart));
  const safeEnd = Math.max(safeStart, Math.min(value.length, selectionEnd));
  const selected = value.slice(safeStart, safeEnd);

  if (selected.length === 0) {
    const insertion = "```\n\n```";
    const nextValue =
      value.slice(0, safeStart) +
      insertion +
      value.slice(safeEnd);

    const cursor = safeStart + 4;

    return {
      value: nextValue,
      selectionStart: cursor,
      selectionEnd: cursor,
    };
  }

  const needsLeadingNewline =
    safeStart > 0 && value[safeStart - 1] !== "\n";
  const needsTrailingNewline =
    safeEnd < value.length && value[safeEnd] !== "\n";

  const prefix = `${needsLeadingNewline ? "\n" : ""}\`\`\`\n`;
  const suffix = `\n\`\`\`${needsTrailingNewline ? "\n" : ""}`;
  const nextValue =
    value.slice(0, safeStart) +
    prefix +
    selected +
    suffix +
    value.slice(safeEnd);

  const selectedStart = safeStart + prefix.length;

  return {
    value: nextValue,
    selectionStart: selectedStart,
    selectionEnd: selectedStart + selected.length,
  };
}
