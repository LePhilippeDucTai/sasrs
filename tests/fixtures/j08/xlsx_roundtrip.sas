/* J08-P1 — PROC EXPORT puis PROC IMPORT DBMS=XLSX : aller-retour.
 * Le classeur est écrit puis relu par le même programme : valeurs,
 * missings et dates (DATE9.) doivent traverser sans perte. */
data work.src;
    length name $8;
    format d date9.;
    name = 'Alice'; score = 95.5; d = 23450; output;
    name = 'Bob';   score = .;    d = .;     output;
    name = 'Carol'; score = 72.3; d = 23376; output;
run;

proc export data=work.src outfile='rt.xlsx' dbms=xlsx replace;
    sheet='SRC';
run;

proc import datafile='rt.xlsx' out=work.back dbms=xlsx;
    sheet='SRC';
run;

proc print data=work.back;
run;
