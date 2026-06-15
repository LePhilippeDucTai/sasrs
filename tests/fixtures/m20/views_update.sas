/* M20.4 — CREATE VIEW, UPDATE ... SET, sous-requêtes dans INSERT */

libname d 'data';

/* Construire une table de travail à partir de sashelp.class */
proc sql;
    create table work.students as
    select name, sex, age, height, weight from d.class;
quit;

/* CREATE VIEW : vue des élèves féminines */
proc sql;
    create view work.girls as
    select name, age, height from work.students
    where sex = 'F';

    title "Vue girls (sex=F)";
    select * from work.girls order by name;
quit;

/* UPDATE ... SET : ajuster le poids, avec et sans WHERE */
proc sql;
    update work.students
    set weight = weight + 5
    where age >= 14;

    title "Après UPDATE weight+5 si age>=14";
    select name, age, weight from work.students
    where age >= 14
    order by name;
quit;

/* INSERT avec sous-requête en FROM (UNION).
   Note : on amorce summary depuis age=15 (inclut William, 7 caractères) pour
   que la longueur de `name` couvre tous les noms insérés ensuite — sinon SAS
   inférerait la longueur sur le premier SELECT, tronquant les noms plus longs
   (limitation connue : CREATE TABLE AS ré-infère la longueur depuis les
   données plutôt que d'hériter de la colonne source). */
proc sql;
    create table work.summary as
    select name, age from work.students where age = 15;

    insert into work.summary
    select name, age from
        (select name, age from work.students where age = 11
         union
         select name, age from work.students where age = 12) as combined;

    title "Summary après INSERT depuis sous-requête FROM(UNION)";
    select * from work.summary order by age, name;
quit;

/* DROP VIEW */
proc sql;
    drop view work.girls;
quit;
