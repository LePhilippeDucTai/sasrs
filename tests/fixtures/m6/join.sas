/* M6 : PROC SQL — CREATE de deux sous-tables puis INNER JOIN sur name
   (eleves a la fois grands ET lourds), refs qualifiees, ORDER BY. */
libname d 'data';

proc sql;
  create table work.tall  as select name, height from d.class where height > 65;
  create table work.heavy as select name, weight from d.class where weight > 100;
  select a.name, a.height, b.weight
  from work.tall as a
  inner join work.heavy as b
  on a.name = b.name
  order by a.name;
quit;
