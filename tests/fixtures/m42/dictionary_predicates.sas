/* M42.1/M42.2: DICTIONARY.MEMBERS (alias of DICTIONARY.TABLES) alongside the
   new CONTAINS / SOUNDS LIKE predicates, against sashelp.class-style data. */

libname d 'data';

proc sql;
  create table work.people as select * from d.class;
quit;

title 'DICTIONARY.MEMBERS lists the same datasets as DICTIONARY.TABLES';
proc sql;
  select libname, memname, nobs, nvar
  from dictionary.members
  order by libname, memname;
quit;

title 'CONTAINS: names holding the substring "ar" (case-sensitive)';
proc sql;
  select name from work.people
  where name contains 'ar'
  order by name;
quit;

title 'NOT CONTAINS: the complement';
proc sql;
  select name from work.people
  where name not contains 'ar'
  order by name;
quit;

title "SOUNDS LIKE 'Rupert': matches Robert (same Soundex code R163)";
proc sql;
  select name from work.people
  where name sounds like 'Rupert'
  order by name;
quit;

title 'NOT SOUNDS LIKE: everyone except that Soundex match';
proc sql;
  select count(*) as n from work.people
  where name not sounds like 'Rupert';
quit;
