/* M14.2 — FILE PRINT + formatted PUT (report écrit dans le listing) */
data _null_;
  infile datalines;
  input name $ age height;
  file print;
  put 'Student: ' name @20 'Age:' age 3. @32 'Ht:' height 6.1;
datalines;
Alfred 14 69.0
Alice 13 56.5
Henry 14 63.5
;
run;
