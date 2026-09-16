export type SnippetPart =
  | {
      type: "text";
      content: string;
    }
  | {
      type: "code";
      content: string;
      language: string;
      index: number;
    };

type LineInfo = {
  start: number;
  contentEnd: number;
  end: number;
  text: string;
};

function readLines(input: string): LineInfo[] {
  const lines: LineInfo[] = [];
  let cursor = 0;

  while (cursor < input.length) {
    const start = cursor;

    while (
      cursor < input.length &&
      input[cursor] !== "\n" &&
      input[cursor] !== "\r"
    ) {
      cursor++;
    }

    const contentEnd = cursor;

    if (input[cursor] === "\r" && input[cursor + 1] === "\n") {
      cursor += 2;
    } else if (
      input[cursor] === "\r" ||
      input[cursor] === "\n"
    ) {
      cursor += 1;
    }

    lines.push({
      start,
      contentEnd,
      end: cursor,
      text: input.slice(start, contentEnd),
    });
  }

  return lines;
}

function openingFence(line: string) {
  const match = line.match(
    /^[ \t]*(`{3,}|~{3,})[ \t]*([^ \t].*?)?[ \t]*$/,
  );

  if (!match) {
    return null;
  }

  const fence = match[1];

  return {
    char: fence[0],
    length: fence.length,
    language: (match[2] ?? "").trim(),
  };
}

function isClosingFence(
  line: string,
  char: string,
  minLength: number,
) {
  const trimmed = line.trim();

  if (trimmed.length < minLength) {
    return false;
  }

  for (const current of trimmed) {
    if (current !== char) {
      return false;
    }
  }

  return true;
}

export function parseSnippetParts(
  content: string,
): SnippetPart[] {
  const lines = readLines(content);
  const parts: SnippetPart[] = [];

  let textStart = 0;
  let codeIndex = 0;
  let lineIndex = 0;

  while (lineIndex < lines.length) {
    const openLine = lines[lineIndex];
    const opening = openingFence(openLine.text);

    if (!opening) {
      lineIndex++;
      continue;
    }

    let closingIndex = lineIndex + 1;

    while (closingIndex < lines.length) {
      if (
        isClosingFence(
          lines[closingIndex].text,
          opening.char,
          opening.length,
        )
      ) {
        break;
      }

      closingIndex++;
    }

    // Unclosed fence: treat it as ordinary text.
    if (closingIndex >= lines.length) {
      lineIndex++;
      continue;
    }

    if (openLine.start > textStart) {
      parts.push({
        type: "text",
        content: content.slice(textStart, openLine.start),
      });
    }

    const bodyStart = openLine.end;
    const closingLine = lines[closingIndex];

    parts.push({
      type: "code",
      language: opening.language,
      content: content.slice(bodyStart, closingLine.start),
      index: codeIndex++,
    });

    textStart = closingLine.end;
    lineIndex = closingIndex + 1;
  }

  if (textStart < content.length) {
    parts.push({
      type: "text",
      content: content.slice(textStart),
    });
  }

  if (parts.length === 0) {
    return [
      {
        type: "text",
        content,
      },
    ];
  }

  return parts;
}

export function fencedCodeBlocks(content: string) {
  return parseSnippetParts(content).filter(
    (
      part,
    ): part is Extract<
      SnippetPart,
      { type: "code" }
    > => part.type === "code",
  );
}
