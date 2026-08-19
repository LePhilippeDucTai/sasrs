/* M43.1 - MIN=/MAX=/DEFAULT=/FUZZ= sur VALUE, avec round-trip CNTLOUT=/CNTLIN=.
   1) DEFAULT=6 : sans largeur explicite au point d'usage, le label est
      justifie a droite sur 6 caracteres (au lieu de son intitule brut).
   2) MIN=/MAX= : une largeur explicite trop etroite/large au point d'usage
      est bornee dans [MIN,MAX].
   3) FUZZ= : une valeur a un cheveu d'une borne inclusive (10) matche quand
      meme la plage superieure, malgre l'ecart flottant.
   4) CNTLOUT= depose les 4 colonnes MIN/MAX/DEFAULT/FUZZ ; CNTLIN= les relit
      a l'identique dans un nouveau catalogue. */

proc format;
  value agef (default=6) low-<21='Minor' 21-high='Adult';
run;

title 'DEFAULT=6 : largeur appliquee sans suffixe explicite au point d''usage';
data work.people;
  input age;
  format age agef.;
  datalines;
5
40
;
run;
proc print data=work.people noobs;
run;

proc format;
  value narrow (min=8 max=10) low-<21='Minor' 21-high='Adult';
run;

title 'MIN=8/MAX=10 : largeur explicite bornee (narrow4. trop etroit -> 8)';
data work.clamped;
  x = put(5, narrow4.);
  y = put(40, narrow20.);
run;
proc print data=work.clamped noobs;
run;

proc format;
  value fuzzy (fuzz=1e-6) low-<10='Below' 10-high='AtLeast';
run;

title 'FUZZ=1e-6 : 9.9999995 (a 5e-7 de 10) matche la plage inclusive AtLeast';
data work.near_boundary;
  input x;
  format x fuzzy.;
  datalines;
9.9999995
9.9
;
run;
proc print data=work.near_boundary noobs;
run;

proc format cntlout=work.fmtctl;
  value agef (min=3 max=10 default=6 fuzz=0.5) low-<21='Minor' 21-high='Adult';
run;

title 'CNTLOUT= depose les colonnes MIN/MAX/DEFAULT/FUZZ';
proc print data=work.fmtctl noobs;
run;

proc format cntlin=work.fmtctl cntlout=work.fmtctl2;
run;

title 'Round-trip CNTLIN->CNTLOUT : FMTCTL2 identique a FMTCTL';
proc compare base=work.fmtctl compare=work.fmtctl2;
run;
