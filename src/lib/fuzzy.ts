/**
 * Minimal fuzzy matcher for the command bar (SKILL.md §15): ordered
 * subsequence matching with lightweight scoring so tighter, earlier and
 * word-boundary matches rank higher.
 */

export interface FuzzyMatch {
  score: number;
  /** Indices of the matched characters inside the target, for highlighting. */
  matchedIndices: number[];
}

const BASE_SCORE = 2;
const BONUS_CONSECUTIVE = 4;
const BONUS_WORD_START = 6;
const BONUS_TARGET_START = 8;
const PENALTY_GAP = -1;

function isWordStart(text: string, index: number): boolean {
  if (index === 0) return true;

  const previous = text[index - 1];
  if (/[\s\-_./\\:]/.test(previous)) return true;

  // camelCase boundary.
  return previous === previous.toLowerCase() && text[index] !== text[index].toLowerCase();
}

export function fuzzyMatch(query: string, target: string): FuzzyMatch | null {
  const trimmedQuery = query.trim();
  if (!trimmedQuery) return { score: 0, matchedIndices: [] };

  const lowerQuery = trimmedQuery.toLowerCase();
  const lowerTarget = target.toLowerCase();
  const matchedIndices: number[] = [];

  let score = 0;
  let searchFrom = 0;
  let previousIndex = -1;

  for (const char of lowerQuery) {
    const index = lowerTarget.indexOf(char, searchFrom);
    if (index === -1) return null;

    matchedIndices.push(index);
    score += BASE_SCORE;

    if (index === 0) {
      score += BONUS_TARGET_START;
    } else if (isWordStart(target, index)) {
      score += BONUS_WORD_START;
    }

    if (previousIndex >= 0) {
      score += index === previousIndex + 1 ? BONUS_CONSECUTIVE : PENALTY_GAP;
    }

    searchFrom = index + 1;
    previousIndex = index;
  }

  // Prefer shorter targets when scores are otherwise equal.
  score -= Math.floor(lowerTarget.length / 8);

  return { score, matchedIndices };
}

export interface RankedResult<T> {
  item: T;
  score: number;
  matchedIndices: number[];
}

/**
 * Ranks items whose text fuzzily matches the query. An empty query keeps the
 * original order with every item included. Sorting is stable, so ties keep
 * their registration order.
 */
export function rankByFuzzy<T>(
  query: string,
  items: readonly T[],
  getText: (item: T) => string,
): RankedResult<T>[] {
  const results: RankedResult<T>[] = [];

  for (const item of items) {
    const match = fuzzyMatch(query, getText(item));
    if (match) {
      results.push({ item, score: match.score, matchedIndices: match.matchedIndices });
    }
  }

  return results.sort((left, right) => right.score - left.score);
}
