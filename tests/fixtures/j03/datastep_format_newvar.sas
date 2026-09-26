/* J03-P6 — FORMAT/ATTRIB sur une variable INCONNUE : la variable est CRÉÉE
 * (comme en SAS). Le dataset de sortie contient newnum (numérique, créée
 * par FORMAT) et newlbl (numérique, créée par ATTRIB, avec son LABEL).
 */

data work.fmtnew;
  length base $8;
  format newnum 8.2;
  attrib newlbl label='Etiquette J03';
  base = 'x';
  newnum = 1.5;
  newlbl = 2;
  output;
run;

title "J03-P6 FORMAT/ATTRIB créent la variable inconnue";
proc print data=work.fmtnew label;
run;
