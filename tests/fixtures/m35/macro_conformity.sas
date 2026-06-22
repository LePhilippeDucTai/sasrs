/* M35.3 - macro conformity: %LENGTH null/empty -> 1, remaining automatic
   variables seeded, and &SYSLAST tracking the last dataset live. */
%put length_empty=%length();
%put length_a=%length(a);
%put length_abc=%length(abc);
%put syscc=&syscc syserr=&syserr sqlobs=&sqlobs;
%put procname=&sysprocessname env=&sysenv;
%put syslast_before=&syslast;
data a;
  x = 1;
run;
%put syslast_after=&syslast;
