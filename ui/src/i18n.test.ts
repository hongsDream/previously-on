/// <reference types="node" />

import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import ts from 'typescript';
import { describe, expect, it } from 'vitest';
import { KOREAN_MESSAGES } from './i18n';

const SOURCE_ROOT = path.resolve(process.cwd(), 'src');
const USER_TEXT_ATTRIBUTES = new Set(['alt', 'aria-label', 'placeholder', 'title']);
const TECHNICAL_TEXT_ALLOWLIST = [
  /^PreviouslyOn$/,
  /^GH$/,
  /^SHA$/,
  /^AuthContext, tenantId$/,
];

describe('Korean product copy catalog', () => {
  const sourceFiles = productSourceFiles(SOURCE_ROOT);

  it('contains a Korean translation for every literal t() key', () => {
    const missing = new Set<string>();
    for (const file of sourceFiles) {
      visitSource(file, (node) => {
        if (!ts.isCallExpression(node) || node.expression.getText() !== 't') return;
        const [message] = node.arguments;
        if (message && ts.isStringLiteralLike(message) && !(message.text in KOREAN_MESSAGES)) {
          missing.add(`${relative(file)}: ${message.text}`);
        }
      });
    }
    expect([...missing]).toEqual([]);
  });

  it('does not leave product-authored English outside the translation function', () => {
    const exposed: string[] = [];
    for (const file of sourceFiles) {
      visitSource(file, (node) => {
        if (ts.isJsxText(node)) record(node.getText(), file, exposed);
        if (ts.isJsxAttribute(node) && USER_TEXT_ATTRIBUTES.has(node.name.getText()) && node.initializer && ts.isStringLiteral(node.initializer)) {
          record(node.initializer.text, file, exposed);
        }
        if (ts.isJsxExpression(node) && node.expression && ts.isStringLiteralLike(node.expression)) {
          record(node.expression.text, file, exposed);
        }
        if (ts.isCallExpression(node) && ['alert', 'confirm'].includes(node.expression.getText())) {
          const [message] = node.arguments;
          if (message && (ts.isStringLiteralLike(message) || ts.isTemplateExpression(message))) {
            record(message.getText(), file, exposed);
          }
        }
      });
    }
    expect(exposed).toEqual([]);
  });

  it('translates dynamic project status values returned by the local API', () => {
    expect(KOREAN_MESSAGES).toMatchObject({
      'file changes were observed, but exact structured PreToolUse/PostToolUse evidence did not match; attribution was downgraded': '파일 변경은 관찰됐지만 정확한 구조화 PreToolUse/PostToolUse 증거가 일치하지 않아 변경 귀속의 신뢰 수준을 낮췄습니다.',
      'Session {value}': '세션 {value}',
      added: '추가됨',
      modified: '수정됨',
      renamed: '이름 변경됨',
      deleted: '삭제됨',
      'temporal revalidation: Unchanged': '시간 기준 재검증: 변경 없음',
      'temporal revalidation: Changed': '시간 기준 재검증: 변경됨',
      'temporal revalidation: Diverged': '시간 기준 재검증: Git 이력 분기',
      'temporal revalidation: Broken': '시간 기준 재검증: 손상됨',
      'temporal revalidation: Degraded': '시간 기준 재검증: 성능 저하',
    });
  });
});

function productSourceFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) return entry.name === 'data' || entry.name === 'test' ? [] : productSourceFiles(entryPath);
    if (!/\.tsx?$/.test(entry.name) || /\.test\.tsx?$/.test(entry.name) || entry.name === 'i18n.tsx') return [];
    return [entryPath];
  });
}

function visitSource(file: string, inspect: (node: ts.Node) => void) {
  const source = ts.createSourceFile(file, readFileSync(file, 'utf8'), ts.ScriptTarget.Latest, true, file.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const visit = (node: ts.Node) => {
    inspect(node);
    ts.forEachChild(node, visit);
  };
  visit(source);
}

function record(value: string, file: string, exposed: string[]) {
  const normalized = value.replace(/\s+/g, ' ').trim();
  if (/^&[a-z]+;$/.test(normalized)) return;
  if (!/[A-Za-z]{2}/.test(normalized)) return;
  if (TECHNICAL_TEXT_ALLOWLIST.some((pattern) => pattern.test(normalized))) return;
  exposed.push(`${relative(file)}: ${normalized}`);
}

function relative(file: string) {
  return path.relative(SOURCE_ROOT, file);
}
