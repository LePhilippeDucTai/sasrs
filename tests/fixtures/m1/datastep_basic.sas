* Etape DATA sans entree : une seule iteration implicite, outputs explicites. ;
data work.squares;
  x = 1; y = x * x; output;
  x = 2; y = x * x; output;
  x = 3; y = x * x; output;
run;

proc print data=work.squares;
run;
