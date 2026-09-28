/* J08-P1 — PROC IMPORT DBMS=XLSX : GETNAMES=NO, SHEET= par numéro,
 * RANGE= en sous-plage, GUESSINGROWS fenêtre d'inférence. */
data work.src;
    length a $4 b $4;
    a = 'r1'; b = 'c1'; x = 1;  output;
    a = 'r2'; b = 'c2'; x = 2;  output;
    a = 'r3'; b = 'c3'; x = 3;  output;
run;

proc export data=work.src outfile='opt.xlsx' dbms=xlsx replace;
    sheet='Data';
run;

/* GETNAMES=NO : noms VAR1..VARn, la ligne d'en-têtes devient donnée. */
proc import datafile='opt.xlsx' out=work.nohead dbms=xlsx;
    getnames=no;
run;

proc contents data=work.nohead;
run;

/* RANGE=B1:C4 : seules les colonnes b et x (en-têtes compris). */
proc import datafile='opt.xlsx' out=work.ranged dbms=xlsx;
    sheet='Data';
    range='B1:C4';
run;

proc print data=work.ranged;
run;

/* GUESSINGROWS=1 : le type est figé sur la première ligne de données. */
proc import datafile='opt.xlsx' out=work.guessed dbms=xlsx;
    guessingrows=1;
run;

proc print data=work.guessed;
run;
