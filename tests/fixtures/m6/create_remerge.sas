/* M6 : PROC SQL — CREATE TABLE AS et REMERGE (agregat + colonne nue, sans
   GROUP BY : chaque ligne porte le max global, et SAS emet une NOTE). */
libname d 'data';

proc sql;
  create table work.tall as
  select name, height, max(height) as max_h
  from d.class
  where sex = 'M';
quit;

title 'Remerged max height (boys)';
proc print data=work.tall;
run;
