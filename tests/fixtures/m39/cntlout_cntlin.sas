/* M39.2 - CNTLOUT=/CNTLIN= : catalogue de formats <-> dataset de controle.
   1) CNTLOUT= depose une observation par plage (+ OTHER) du format AGEF dans
      WORK.FMTCTL - colonnes FMTNAME/START/END/LABEL/TYPE/SEXCL/EEXCL/HLO.
   2) PROC PRINT du dataset de controle (verification visuelle des colonnes).
   3) CNTLIN=work.fmtctl reconstruit AGEF dans un NOUVEAU nom (AGEF2) via un
      second PROC FORMAT, qui redepose aussitot (CNTLOUT=) dans WORK.FMTCTL2 -
      oracle round-trip : FMTCTL et FMTCTL2 doivent etre identiques (PROC
      COMPARE : "No unequal values").
   4) Le format reconstruit (AGEF, ecrase par le CNTLIN= au meme nom) est
      utilise dans une etape DATA + PROC PRINT : rendu identique a l'original. */

proc format cntlout=work.fmtctl;
  value agef low-<21='Minor' 21-high='Adult' other='Unknown';
run;

title "Dataset de controle produit par CNTLOUT=";
proc print data=work.fmtctl noobs;
run;
title;

/* Round-trip : CNTLIN= relit FMTCTL et redefinit AGEF ; CNTLOUT= redepose
   aussitot le catalogue resultant dans FMTCTL2. */
proc format cntlin=work.fmtctl cntlout=work.fmtctl2;
run;

title "Comparaison FMTCTL (original) vs FMTCTL2 (round-trip CNTLIN->CNTLOUT)";
proc compare base=work.fmtctl compare=work.fmtctl2;
run;
title;

/* Le format AGEF, reconstruit par le CNTLIN= ci-dessus, resout exactement
   comme l'original. */
data work.people;
  input age;
  format age agef.;
  datalines;
5
21
40
;
run;

title "Format AGEF reconstruit par CNTLIN= applique a des donnees";
proc print data=work.people noobs;
run;
title;
