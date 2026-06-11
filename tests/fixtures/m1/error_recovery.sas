* La session continue apres une PROC inconnue et un statement invalide. ;
proc nonexistent data=work.x;
run;

data work.a;
  x = 10;
run;

frobnicate this;

proc print data=work.a;
run;
