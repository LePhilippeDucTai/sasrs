/* M14.1 — DATALINES + list input + PROC PRINT */
data pets;
  input name $ species $ age weight;
datalines;
Rex Dog 5 30.5
Felix Cat 3 4.2
Tweety Bird 1 0.1
;
run;

proc print data=pets;
run;
