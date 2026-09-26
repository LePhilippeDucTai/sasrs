/* J03-P6 — Divergences des fonctions caractères, valeurs attendues tirées
 * de la doc SAS (SAS 9.4 Functions Reference, lefunctionsref) :
 * https://documentation.sas.com/doc/en/pgmsascdc/v_038/lefunctionsref/
 *
 * FIND : la recherche COMMENCE à la position de départ (INCLUSE) —
 *   doc : find('abc','a') = 1 ; find(xyz,'she',22) = 27 pour
 *   xyz='She sells seashells? Yes, she does.'
 * COMPBL : chaque occurrence de 2 espaces ou plus devient UN espace ; un
 *   blanc isolé n'est pas affecté ; les tabulations ne sont pas des blancs
 *   (fonction de niveau I18N 0, SBCS).
 * STRIP/CATS/CATX : suppression des blancs (espaces 0x20) de tête et de
 *   queue, PAS des tabulations (I18N niveau 0).
 * REPEAT : plafond compté en CARACTÈRES (32 767), pas en octets —
 *   cohérent avec les largeurs en caractères de J03-P3/P4/P5.
 */

data work.charfns;
  length s $40 tabstr $3;
  length compbl_collapse $16 compbl_edges $16 compbl_tab $16 repeat_e $3;
  s = 'She sells seashells? Yes, she does.';
  tabstr = 'a	b';
  tab = substr(tabstr, 2, 1);

  find_abc_a   = find('abc', 'a');
  find_start22 = find(s, 'she', 22);
  find_start5  = find('hello world', 'o', 5);

  compbl_collapse = compbl('125 E  Main St');
  compbl_edges    = compbl('  hello world  ');
  compbl_tab      = compbl('hello  ' || tab || '  world');

  strip_spaces = '*' || strip('   hello   ') || '*';
  strip_tab    = '*' || strip(tab || 'hello' || tab) || '*';

  cats_mix  = cats(' a ', tab || 'b' || tab);
  catx_mix  = catx('-', ' a ', tab || 'b' || tab);

  repeat_e     = repeat('é', 3);
  /* Plafond en CARACTÈRES : 32 767 caractères de « é » (2 octets chacun),
   * soit 65 534 octets — l'ancien plafond octet donnait 16 383 caractères. */
  repeat_chars = length(repeat('é', 1e9));
run;

title "J03-P6 fonctions caractères — doc lefunctionsref";
proc print data=work.charfns;
run;
