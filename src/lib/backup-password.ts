/**
 * Validation de la phrase secrète pour l'export chiffré.
 *
 * Fonction pure — se teste pour de vrai.
 *
 * Elle rend un CODE de refus, jamais une phrase ni une clé de catalogue : ce
 * module est un `.ts` pur que `test-backup-wiring.ts` charge à l'exécution sous
 * tsx, où un `import { t }` casserait la suite (contrainte de module I18N-28).
 * Le point d'appel choisit la clé — même forme que `logCountLabel` à I18N-29.
 *
 * L'union fermée n'est pas décorative : elle rend IMPOSSIBLE à la compilation
 * qu'un motif de refus réémette la phrase saisie ou sa confirmation. C'est ce
 * qui remplace les trois assertions de fuite que ce round rend vacantes.
 */

export type PasswordRefusal = "required" | "mismatch";

export type PasswordCheck =
  | { ok: true; password: string }
  | { ok: false; reason: PasswordRefusal };

export function checkExportPassword(
  enabled: boolean,
  phrase: string,
  confirm: string,
): PasswordCheck {
  if (!enabled) {
    return { ok: true, password: "" };
  }

  if (phrase === "") {
    return { ok: false, reason: "required" };
  }

  if (phrase !== confirm) {
    return { ok: false, reason: "mismatch" };
  }

  return { ok: true, password: phrase };
}
