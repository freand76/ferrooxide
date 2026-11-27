// Names in class methods: a method calls itself recursively although its own name is also
// its result variable; an automatic method passes its own `ref` argument on to a
// recursive call; a variable declared in one method (or block) does not hide a property
// of the same name in another; variables of a type parameter's type declared in blocks;
// foreach over associative array properties with class handle and enum keys, named
// without `this.`; a queue of enums declared in a block; an unused parameterized class
// whose method could not be compiled with its default parameters (IEEE 1800-2017 8.25).
typedef enum {INFO, WARN, ERR} sev_t;
class M;
  int id;
  function new(int i);
    id = i;
  endfunction
  function int get_id();
    return id;
  endfunction
endclass

virtual class F;
  pure virtual function int find(string n, int d);
  virtual function string kind();
    return "F";
  endfunction
endclass
class DF extends F;
  extern virtual function int find(string n, int d);
endclass
function int DF::find(string n, int d);
  int o;
  if (d > 0) begin
    if (o == 0) o = find(n, d - 1);
  end
  return o + 1;
endfunction
// (never used: T::make() of the default int would be an error)
class Gen #(type T = int) extends F;
  virtual function int find(string n, int d);
    return T::make();
  endfunction
endclass

class E;
  E kids[$];
  int id;
  function new(int i);
    id = i;
  endfunction
  function void all(ref E out[$], input bit rec);
    foreach (kids[i]) begin
      out.push_back(kids[i]);
      if (rec) kids[i].all(out, 1);
    end
  endfunction
endclass

class B;
  protected bit maps[M];
  int cnt[sev_t];
  function void add(M m);
    maps[m] = 1;
  endfunction
  function M first();
    M maps[$];  // a local of the same name as the property
    foreach (this.maps[m]) maps.push_back(m);
    return maps[0];
  endfunction
  extern function int sum();
  function string report();
    string r;
    foreach (cnt[s]) r = {r, $sformatf("%s=%0d ", s.name(), cnt[s])};
    return r;
  endfunction
endclass
function int B::sum();
  int s;
  foreach (maps[i]) begin
    M m = i;
    s += m.get_id() + i.get_id();
  end
  return s;
endfunction

class P #(type T = int);
  function int nulls();
    int n;
    begin
      T d;
      n += d == null;
    end
    if (1) begin
      T d2;
      n += d2 == null;
    end
    return n;
  endfunction
endclass

module t;
  integer errors = 0;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask

  DF df;
  F f;
  E r, x, y, z;
  E l[$];
  B b;
  M m3, m4;
  P #(M) p;
  string s;

  initial begin
    df = new;
    f = df;
    check(`__LINE__, f.find("x", 3), 4);
    r = new(0); x = new(1); y = new(2); z = new(3);
    r.kids = '{x, y};
    x.kids = '{z};
    r.all(l, 1);
    check(`__LINE__, l.size(), 3);
    check(`__LINE__, l[0].id * 100 + l[1].id * 10 + l[2].id, 132);
    b = new;
    m3 = new(3);
    m4 = new(4);
    b.add(m3);
    b.add(m4);
    check(`__LINE__, b.sum(), 14);
    check(`__LINE__, b.first() != null, 1);
    b.cnt[ERR] = 2;
    b.cnt[INFO] = 5;
    s = b.report();
    if (s != "INFO=5 ERR=2 ") begin
      $display("FAILED line %0d: %s", `__LINE__, s);
      errors = errors + 1;
    end
    begin
      sev_t q[$];
      q.push_back(WARN);
      q.push_back(ERR);
      foreach (q[i]) b.cnt[q[i]] = 10 + i;
    end
    check(`__LINE__, b.cnt[WARN], 10);
    check(`__LINE__, b.cnt[ERR], 11);
    p = new;
    check(`__LINE__, p.nulls(), 2);
    if (errors == 0) $display("PASSED");
  end
endmodule
