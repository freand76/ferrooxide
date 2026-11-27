// Methods of a struct's members: a string's, and a class handle's (IEEE 1800-2017 6.16,
// 8.4), also of an array element's member.
module top;
  class C;
    int v = 7;
    function int get(); return v; endfunction
  endclass
  typedef struct {
    string name;
    C obj;
  } row_t;
  row_t row;
  row_t rows[2];
  int errors = 0;
  initial begin
    row.name = "abcd";
    row.obj = new;
    rows[1].name = "xy";
    if (row.name.len() != 4) errors++;
    if (row.obj.get() != 7) errors++;
    if (row.name.substr(1, 2) != "bc") errors++;
    if (rows[1].name.len() != 2) errors++;
    if (errors == 0) $display("PASSED"); else $display("FAILED: %0d", errors);
  end
endmodule
