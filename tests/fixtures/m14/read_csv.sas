/* M14.3 — PROC IMPORT (CSV) + PROC PRINT */
proc import datafile='data/pets.csv' out=work.pets dbms=csv replace;
  getnames=yes;
run;

proc print data=work.pets;
run;
