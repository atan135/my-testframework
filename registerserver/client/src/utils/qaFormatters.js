export function formatParameters(parameters = []) {
  if (parameters.length === 0) {
    return '无';
  }
  return parameters.map((parameter) => `${parameter.name}: ${parameter.type}`).join(', ');
}

export function parameterLabel(parameter) {
  return `${parameter.name} (${parameter.type})`;
}

export function historyStatusType(status) {
  if (status === 'success') return 'success';
  if (status === 'failed') return 'danger';
  return 'warning';
}

export function formatTime(value) {
  if (!value) {
    return '-';
  }
  return new Date(value).toLocaleString();
}

export function formatOutputPreview(row, maxLength = 160) {
  const value = primaryOutputValue(row);
  if (!hasOutputValue(value)) {
    return '-';
  }

  return truncateText(normalizeOutputValue(value).preview, maxLength);
}

export function formatOutputDetails(row) {
  if (!row) {
    return '-';
  }

  const error = row.error;
  const result = row.result;
  const hasError = hasOutputValue(error);
  const hasResult = hasOutputValue(result);

  if (hasError && hasResult) {
    return [
      'Error:',
      normalizeOutputValue(error).details,
      '',
      'Result:',
      normalizeOutputValue(result).details,
    ].join('\n');
  }

  if (hasError) {
    return normalizeOutputValue(error).details;
  }

  if (hasResult) {
    return normalizeOutputValue(result).details;
  }

  return '-';
}

function primaryOutputValue(row) {
  if (!row) {
    return '';
  }

  if (hasOutputValue(row.error)) {
    return row.error;
  }

  return row.result;
}

function normalizeOutputValue(value) {
  const parsed = parseJsonLike(value);
  const outputValue = parsed.parsed ? parsed.value : value;

  if (isStructuredOutput(outputValue)) {
    const pretty = JSON.stringify(outputValue, null, 2);
    return {
      preview: toSingleLine(JSON.stringify(outputValue)),
      details: pretty,
    };
  }

  if (typeof outputValue === 'string') {
    const decoded = decodeUnicodeEscapes(outputValue.trim());
    return {
      preview: toSingleLine(decoded),
      details: decoded || '-',
    };
  }

  const text = String(outputValue ?? '');
  return {
    preview: toSingleLine(text),
    details: text || '-',
  };
}

function parseJsonLike(value) {
  if (isStructuredOutput(value)) {
    return { parsed: true, value };
  }

  let current = value;
  let parsed = false;
  for (let index = 0; index < 3; index += 1) {
    if (typeof current !== 'string') {
      break;
    }

    const trimmed = current.trim();
    if (!looksLikeJson(trimmed)) {
      break;
    }

    try {
      current = JSON.parse(trimmed);
      parsed = true;
    } catch {
      break;
    }
  }

  return { parsed, value: current };
}

function looksLikeJson(value) {
  if (!value) {
    return false;
  }

  return (
    (value.startsWith('{') && value.endsWith('}')) ||
    (value.startsWith('[') && value.endsWith(']')) ||
    (value.startsWith('"') && value.endsWith('"')) ||
    value === 'true' ||
    value === 'false' ||
    value === 'null'
  );
}

function isStructuredOutput(value) {
  return Array.isArray(value) || (value !== null && typeof value === 'object');
}

function hasOutputValue(value) {
  if (value === null || value === undefined) {
    return false;
  }

  return typeof value !== 'string' || value.trim().length > 0;
}

function decodeUnicodeEscapes(value) {
  return value.replace(/\\u([0-9a-fA-F]{4})/g, (_, hex) => String.fromCharCode(Number.parseInt(hex, 16)));
}

function toSingleLine(value) {
  return String(value || '').replace(/\s+/g, ' ').trim();
}

function truncateText(value, maxLength) {
  if (value.length <= maxLength) {
    return value;
  }

  return `${value.slice(0, Math.max(0, maxLength - 3))}...`;
}
