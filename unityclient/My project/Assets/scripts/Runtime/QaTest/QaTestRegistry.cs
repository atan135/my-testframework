using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using UnityEngine;

namespace QaTestFramework
{
    internal sealed class QaTestRegistry
    {
        private const string RegistryLogPrefix = "[QaTest][Registry] ";
        private readonly Dictionary<string, QaTestMethodEntry> methods = new Dictionary<string, QaTestMethodEntry>();
        private readonly Dictionary<string, QaTestMethodEntry> methodsByFullId = new Dictionary<string, QaTestMethodEntry>();

        public IReadOnlyCollection<QaTestMethodEntry> Methods
        {
            get { return methods.Values; }
        }

        public void Refresh()
        {
            Assembly[] loadedAssemblies = AppDomain.CurrentDomain.GetAssemblies();
            Debug.Log(RegistryLogPrefix + "QaTestRegistry.Refresh all assemblies count=" + (loadedAssemblies != null ? loadedAssemblies.Length : 0));
            Refresh(loadedAssemblies);
        }

        public void Refresh(IEnumerable<Assembly> assemblies)
        {
            Debug.Log(RegistryLogPrefix + "QaTestRegistry.Refresh begin.");
            methods.Clear();
            methodsByFullId.Clear();
            List<QaTestMethodCandidate> candidates = new List<QaTestMethodCandidate>();

            IEnumerable<Assembly> scanAssemblies = assemblies ?? AppDomain.CurrentDomain.GetAssemblies();
            int assemblyIndex = 0;
            foreach (Assembly assembly in scanAssemblies)
            {
                if (assembly == null)
                {
                    Debug.Log(RegistryLogPrefix + "Scan assembly[" + assemblyIndex + "] skipped: null.");
                    assemblyIndex++;
                    continue;
                }

                string assemblyName = FormatAssemblyName(assembly);
                Debug.Log(RegistryLogPrefix + "Scan assembly[" + assemblyIndex + "] begin: " + assemblyName);
                Type[] types = SafeGetTypes(assembly);
                Debug.Log(RegistryLogPrefix + "Scan assembly[" + assemblyIndex + "] typesCount=" + (types != null ? types.Length : 0) + ": " + assemblyName);

                int typeIndex = 0;
                foreach (Type type in types)
                {
                    CollectType(type, assemblyName, typeIndex, candidates);
                    typeIndex++;
                }

                Debug.Log(RegistryLogPrefix + "Scan assembly[" + assemblyIndex + "] end: " + assemblyName + " candidates=" + candidates.Count);
                assemblyIndex++;
            }

            Debug.Log(RegistryLogPrefix + "ValidateUniqueMethodNames begin candidateCount=" + candidates.Count);
            ValidateUniqueMethodNames(candidates);
            Debug.Log(RegistryLogPrefix + "ValidateUniqueMethodNames end.");

            int candidateIndex = 0;
            foreach (QaTestMethodCandidate candidate in candidates)
            {
                Debug.Log(RegistryLogPrefix + "Register candidate[" + candidateIndex + "] begin: " + BuildDefinitionMethodId(candidate.Method));
                RegisterMethod(candidate.Method, candidate.Attribute);
                Debug.Log(RegistryLogPrefix + "Register candidate[" + candidateIndex + "] end: " + BuildDefinitionMethodId(candidate.Method));
                candidateIndex++;
            }

            Debug.Log(RegistryLogPrefix + "QaTestRegistry.Refresh end methods=" + methods.Count + " fullMethods=" + methodsByFullId.Count);
        }

        public bool TryGet(string methodId, out QaTestMethodEntry entry)
        {
            if (!string.IsNullOrWhiteSpace(methodId) && methods.TryGetValue(methodId, out entry))
            {
                return true;
            }

            if (!string.IsNullOrWhiteSpace(methodId) && methodsByFullId.TryGetValue(methodId, out entry))
            {
                return true;
            }

            entry = methods.Values.FirstOrDefault(method =>
                method.Method.Name == methodId ||
                method.DisplayName == methodId ||
                method.Id == methodId ||
                method.FullId == methodId);
            return entry != null;
        }

        public QaTestMethodDto[] ToDtos()
        {
            return methods.Values
                .OrderBy(method => method.Method.DeclaringType != null ? method.Method.DeclaringType.FullName : string.Empty)
                .ThenBy(method => method.Method.Name)
                .Select(method => method.ToDto())
                .ToArray();
        }

        private void CollectType(Type type, string assemblyName, int typeIndex, List<QaTestMethodCandidate> candidates)
        {
            if (type == null)
            {
                Debug.Log(RegistryLogPrefix + "Collect type[" + typeIndex + "] skipped: null in " + assemblyName);
                return;
            }

            string typeName = FormatTypeName(type);
            Debug.Log(RegistryLogPrefix + "Collect type[" + typeIndex + "] begin: " + typeName + " in " + assemblyName);
            BindingFlags flags = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static | BindingFlags.Instance | BindingFlags.DeclaredOnly;
            MethodInfo[] methodsOnType = type.GetMethods(flags);
            Debug.Log(RegistryLogPrefix + "Collect type[" + typeIndex + "] methodCount=" + (methodsOnType != null ? methodsOnType.Length : 0) + ": " + typeName);

            int methodIndex = 0;
            foreach (MethodInfo method in methodsOnType)
            {
                string methodId = BuildDefinitionMethodId(method);
                Debug.Log(RegistryLogPrefix + "Inspect method[" + methodIndex + "] begin: " + methodId);
                QaTestAttribute attribute = method.GetCustomAttribute<QaTestAttribute>(true);
                if (attribute == null || method.ContainsGenericParameters)
                {
                    Debug.Log(
                        RegistryLogPrefix +
                        "Inspect method[" + methodIndex + "] skip: " + methodId +
                        " attribute=" + (attribute != null) +
                        " containsGenericParameters=" + method.ContainsGenericParameters);
                    methodIndex++;
                    continue;
                }

                Debug.Log(RegistryLogPrefix + "Inspect method[" + methodIndex + "] candidate: " + methodId);
                candidates.Add(new QaTestMethodCandidate(method, attribute));
                methodIndex++;
            }

            Debug.Log(RegistryLogPrefix + "Collect type[" + typeIndex + "] end: " + typeName + " candidates=" + candidates.Count);
        }

        private static void ValidateUniqueMethodNames(IEnumerable<QaTestMethodCandidate> candidates)
        {
            Dictionary<string, MethodInfo> methodsByName = new Dictionary<string, MethodInfo>();
            foreach (QaTestMethodCandidate candidate in candidates)
            {
                string methodName = candidate.Method.Name;
                if (methodsByName.TryGetValue(methodName, out MethodInfo existing))
                {
                    throw new InvalidOperationException(
                        "Duplicate QaTest method name: " + methodName +
                        ". QaTest method names must be globally unique. Existing=" + BuildDefinitionMethodId(existing) +
                        ", Duplicate=" + BuildDefinitionMethodId(candidate.Method));
                }

                methodsByName[methodName] = candidate.Method;
            }
        }

        private void RegisterMethod(MethodInfo method, QaTestAttribute attribute)
        {
            if (method.IsStatic)
            {
                AddMethod(method, null, attribute);
                return;
            }

            Type declaringType = method.DeclaringType;
            if (declaringType == null || !typeof(MonoBehaviour).IsAssignableFrom(declaringType))
            {
                Debug.Log(RegistryLogPrefix + "Register non-static skipped because declaring type is not MonoBehaviour: " + BuildDefinitionMethodId(method));
                return;
            }

            Debug.Log(RegistryLogPrefix + "FindObjectsOfType begin: " + declaringType.FullName);
            UnityEngine.Object[] targets = UnityEngine.Object.FindObjectsOfType(declaringType, true);
            Debug.Log(RegistryLogPrefix + "FindObjectsOfType end: " + declaringType.FullName + " count=" + (targets != null ? targets.Length : 0));
            if (targets.Length > 1)
            {
                throw new InvalidOperationException(
                    "Multiple QaTest targets found for method: " + BuildDefinitionMethodId(method) +
                    ". Instance QaTest methods must resolve to a single target when short method IDs are enabled.");
            }

            foreach (UnityEngine.Object target in targets)
            {
                AddMethod(method, target, attribute);
            }
        }

        private void AddMethod(MethodInfo method, object target, QaTestAttribute attribute)
        {
            string methodName = method.Name;
            QaTestMethodEntry duplicate = methods.Values.FirstOrDefault(entry => entry.Method.Name == methodName);
            if (duplicate != null)
            {
                throw new InvalidOperationException(
                    "Duplicate QaTest method name: " + methodName +
                    ". QaTest method names must be globally unique. Existing=" + duplicate.FullId +
                    ", Duplicate=" + BuildFullMethodId(method, target));
            }

            string id = BuildShortMethodId(method);
            string fullId = BuildFullMethodId(method, target);
            if (methods.ContainsKey(id))
            {
                throw new InvalidOperationException(
                    "Duplicate QaTest method signature: " + id +
                    ". QaTest short method signatures must be globally unique. Existing=" + methods[id].FullId +
                    ", Duplicate=" + fullId);
            }

            QaTestMethodEntry entry = new QaTestMethodEntry(id, fullId, method, target, attribute);
            methods[id] = entry;
            methodsByFullId[fullId] = entry;
            Debug.Log(RegistryLogPrefix + "AddMethod success id=" + id + " fullId=" + fullId);
        }

        private static string BuildShortMethodId(MethodInfo method)
        {
            string parameters = string.Join(",", method.GetParameters().Select(parameter => parameter.ParameterType.FullName));
            return method.Name + "(" + parameters + ")";
        }

        private static string BuildDefinitionMethodId(MethodInfo method)
        {
            string declaringTypeName = method.DeclaringType != null ? method.DeclaringType.FullName : "UnknownType";
            string parameters = string.Join(",", method.GetParameters().Select(parameter => parameter.ParameterType.FullName));
            return declaringTypeName + "." + method.Name + "(" + parameters + ")";
        }

        private static string BuildFullMethodId(MethodInfo method, object target)
        {
            string id = BuildDefinitionMethodId(method);

            UnityEngine.Object unityTarget = target as UnityEngine.Object;
            if (unityTarget != null)
            {
                id += "@" + unityTarget.GetInstanceID();
            }

            return id;
        }

        private static Type[] SafeGetTypes(Assembly assembly)
        {
            string assemblyName = FormatAssemblyName(assembly);
            Debug.Log(RegistryLogPrefix + "SafeGetTypes begin: " + assemblyName);
            try
            {
                Type[] types = assembly.GetTypes();
                Debug.Log(RegistryLogPrefix + "SafeGetTypes end: " + assemblyName + " count=" + (types != null ? types.Length : 0));
                return types ?? Array.Empty<Type>();
            }
            catch (ReflectionTypeLoadException exception)
            {
                Type[] types = exception.Types.Where(type => type != null).ToArray();
                Debug.LogWarning(
                    RegistryLogPrefix +
                    "SafeGetTypes ReflectionTypeLoadException: " + assemblyName +
                    " loadedCount=" + types.Length +
                    " loaderExceptionCount=" + (exception.LoaderExceptions != null ? exception.LoaderExceptions.Length : 0));
                return types;
            }
            catch (Exception exception)
            {
                Debug.LogWarning(RegistryLogPrefix + "SafeGetTypes failed: " + assemblyName + " " + exception.GetType().Name + ": " + exception.Message);
                return Array.Empty<Type>();
            }
        }

        private static string FormatAssemblyName(Assembly assembly)
        {
            if (assembly == null)
            {
                return "<null>";
            }

            try
            {
                AssemblyName name = assembly.GetName();
                return name != null ? name.Name : "<unknown>";
            }
            catch (Exception exception)
            {
                return "<assembly-name-error:" + exception.GetType().Name + ">";
            }
        }

        private static string FormatTypeName(Type type)
        {
            if (type == null)
            {
                return "<null>";
            }

            return string.IsNullOrEmpty(type.FullName) ? type.Name : type.FullName;
        }

        private sealed class QaTestMethodCandidate
        {
            public QaTestMethodCandidate(MethodInfo method, QaTestAttribute attribute)
            {
                Method = method;
                Attribute = attribute;
            }

            public MethodInfo Method { get; }
            public QaTestAttribute Attribute { get; }
        }
    }
}
