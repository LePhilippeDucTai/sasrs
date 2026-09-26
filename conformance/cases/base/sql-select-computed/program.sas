/* PROC SQL CREATE TABLE AS SELECT + ORDER BY — cf. PROC SQL documentation. */
libname ind 'data';

proc sql;
  create table scaled as
    select id, amount, amount * 2 as double_amount
    from ind.orders
    order by id, amount;
quit;
