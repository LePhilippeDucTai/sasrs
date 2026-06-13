/* M4 : PROC FORMAT (format caractere $sexf et format numerique a plages
   agegrp) applique via le statement FORMAT en PROC PRINT, puis PROC CONTENTS
   (metadonnees, dont les formats attaches). */
libname d 'data';

proc format;
  value $sexf 'M'='Male' 'F'='Female';
  value agegrp low-12='Child' 13-15='Teen' 16-high='Adult';
run;

data work.people;
  set d.class(keep=name sex age);
  format sex $sexf. age agegrp.;
run;

title 'User formats applied';
proc print data=work.people;
run;

title 'Metadata';
proc contents data=work.people;
run;
