/* J02-P7 : contrat PROC TABULATE / PROC REPORT de bout en bout.
   Corrections directes (doc SAS 9.4) : niveaux CLASS et groupes REPORT sur
   la valeur formatee (format stocke), observations a CLASS manquante exclues
   de toute la table (MISSING les garde), ORDER sans consolidation, references
   COMPUTE par nom d'element (jamais par libelle).
   Diagnostics : WARNING d'affichage (KEYLABEL, HEADLINE, option OL de BREAK),
   ERROR avant toute sortie (deuxieme TABLE, fonction dans WHERE, COMPUTE
   BEFORE). */
libname d 'data';

proc format;
  value agegrp low-12 = 'Pre-teen' 13-high = 'Teen';
run;

data class;
  set d.class;
  format age agegrp.;
  if name = 'Alfred' then sex = ' ';
run;

title 'TABULATE: formatted CLASS levels, Alfred (blank sex) excluded';
proc tabulate data=class;
  class sex age;
  var height;
  table age all, height*(n mean) pctn;
run;

title 'TABULATE MISSING: the blank sex is a level';
proc tabulate data=class missing;
  class sex;
  keylabel n='Count';
  table sex all, n;
run;

title 'TABULATE: a second TABLE statement is rejected';
proc tabulate data=class;
  class sex;
  table sex;
  table sex, n;
run;

title 'REPORT: ORDER keeps one row per observation';
proc report data=d.class nowd headline;
  column sex name height;
  where age > 14;
  define sex / order;
  define height / analysis sum format=6.1;
  break after sex / summarize ol;
run;

title 'REPORT: GROUP on the stored format of age';
proc report data=class nowd;
  column age height;
  define age / group;
  define height / analysis mean format=6.2;
run;

title 'REPORT: COMPUTE refers to a labelled item by its name';
proc report data=d.class nowd;
  column name height hcm;
  where age = 11;
  define height / display 'Height (in)';
  define hcm / computed format=6.1 'Height (cm)';
  compute hcm;
    hcm = height * 2.54;
  endcomp;
run;

title 'REPORT: rejected before any output';
proc report data=d.class nowd;
  column name sex;
  where upcase(sex) = 'F';
run;

proc report data=d.class nowd;
  column sex height;
  define sex / group;
  compute before;
    line 'Heights by sex';
  endcomp;
run;
title;
