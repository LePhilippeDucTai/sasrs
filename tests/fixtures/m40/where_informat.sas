/* M40.3 - WHERE statement standalone + INFORMAT statement.
   1) Oracle de la case : `where x > 2;` produit EXACTEMENT la meme sortie
      (obs ET compte de la NOTE de lecture) que l'option de dataset
      `set a(where=(x > 2))`.
   2) Le filtre est pre-PDV : _N_ ne compte que les obs retenues.
   3) INFORMAT statement : `informat d date9.;` fait lire d par l'INPUT en
      mode liste comme une date SAS (modified list input) ; le FORMAT associe
      restitue la date en listing. */

data work.a;
  input x;
  datalines;
1
2
3
4
5
;
run;

data work.stmt;
  set work.a;
  where x > 2;
  n = _n_;
run;

data work.opt;
  set work.a(where=(x > 2));
  n = _n_;
run;

title "WHERE statement : obs filtrees avant le PDV, _N_ suit le flux filtre";
proc print data=work.stmt noobs;
run;
title;

title "Oracle : WHERE= option, sortie identique au WHERE statement";
proc print data=work.opt noobs;
run;
title;

proc compare base=work.stmt compare=work.opt;
run;

data work.dates;
  informat d date9.;
  format d date9.;
  input name $ d;
  datalines;
alpha 01JAN2020
beta 15MAR2021
gamma 31DEC2022
;
run;

title "INFORMAT statement : lecture date9. en INPUT liste";
proc print data=work.dates noobs;
run;
title;
